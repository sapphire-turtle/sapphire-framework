//! The framework's tools.
//!
//! The same vocabulary as the framework's half of the CLI, cut to what an agent should be
//! handed: reading the server's state, the workgroup's, and the workspace's files. The
//! commands that change who may connect or where the workspace lives (`device invite` /
//! `retire`, `external-device`, `workgroup join`, `workspace select` / `move` / `remove`,
//! `service`) stay with the human at the CLI.
//!
//! Writing the workspace's files is a separate set, [`workspace_write_tools`]: most apps
//! keep their own invariants in those files (an entry's front matter, a ledger's balance)
//! and give the agent tools that keep them, so raw writes are an application's choice.
//!
//! Both sets are generic over the application's handler `S`: a tool finds its
//! [`ToolContext`] through `S: AsRef<ToolContext>`, so an application adds them to its own
//! `ToolRouter<S>` beside its own tools.

use std::path::PathBuf;

use rmcp::ErrorData;
use rmcp::handler::server::common::{schema_for_empty_input, schema_for_type};
use rmcp::handler::server::router::tool::{ToolRoute, ToolRouter};
use rmcp::handler::server::tool::ToolCallContext;
use rmcp::model::{CallToolResult, ContentBlock, JsonObject, Tool, ToolAnnotations};
use sapphire_server_api as proto;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::context::ToolContext;
use crate::error::{Error, Result};

/// How many search hits a `workspace_search` returns when the agent names no limit.
pub const DEFAULT_SEARCH_LIMIT: usize = 10;

/// The framework's read-only tools: `status`, `workspace_show`, `workspace_search`,
/// `workspace_read_file`, `workspace_list_dir`, `workgroup_show` and `device_list`.
pub fn framework_tools<S>() -> ToolRouter<S>
where
    S: AsRef<ToolContext> + Send + Sync + 'static,
{
    ToolRouter::new()
        .with_route(route::<S, Empty, _, _>(
            tool(
                "status",
                "Report the application server: its version, process id, how it was started, \
                 and the application's own status rows.",
                None,
            )
            .annotate(read_only()),
            |ctx, Empty {}| async move {
                let report: proto::StatusReport =
                    ctx.call(proto::SERVER_INFO, serde_json::json!({})).await?;
                json(&report)
            },
        ))
        .with_route(route::<S, Empty, _, _>(
            tool(
                "workspace_show",
                "Show the workspace the server serves: its root on this host, its id, whether \
                 it is reachable, and its sync state.",
                None,
            )
            .annotate(read_only()),
            |ctx, Empty {}| async move {
                let current: proto::WorkspaceCurrentResult = ctx
                    .call(proto::WORKSPACE_CURRENT, serde_json::json!({}))
                    .await?;
                json(&current)
            },
        ))
        .with_route(route::<S, SearchArgs, _, _>(
            tool(
                "workspace_search",
                "Search the workspace's files. Returns the best-matching files first, each \
                 with its workspace-relative path, a score and a one-line snippet.",
                Some(schema_for_type::<SearchArgs>()),
            )
            .annotate(read_only()),
            |ctx, args: SearchArgs| async move {
                let result: proto::SearchResult = ctx
                    .call(
                        proto::SEARCH,
                        proto::SearchParams {
                            query: args.query,
                            limit: args.limit.unwrap_or(DEFAULT_SEARCH_LIMIT),
                            mode: args.mode.unwrap_or_default().into(),
                        },
                    )
                    .await?;
                json(&result.hits)
            },
        ))
        .with_route(route::<S, PathArgs, _, _>(
            tool(
                "workspace_read_file",
                "Read a text file of the workspace in full.",
                Some(schema_for_type::<PathArgs>()),
            )
            .annotate(read_only()),
            |ctx, args: PathArgs| async move {
                let result: proto::ReadResult = ctx
                    .call(proto::READ_FILE, proto::PathParams { path: args.path })
                    .await?;
                Ok(CallToolResult::success(vec![ContentBlock::text(
                    result.content,
                )]))
            },
        ))
        .with_route(route::<S, ListDirArgs, _, _>(
            tool(
                "workspace_list_dir",
                "List the direct children of a directory of the workspace.",
                Some(schema_for_type::<ListDirArgs>()),
            )
            .annotate(read_only()),
            |ctx, args: ListDirArgs| async move {
                let result: proto::ListDirResult = ctx
                    .call(
                        proto::LIST_DIR,
                        proto::PathParams {
                            path: args.path.unwrap_or_default(),
                        },
                    )
                    .await?;
                json(&result.entries)
            },
        ))
        .with_route(route::<S, Empty, _, _>(
            tool(
                "workgroup_show",
                "Show the workgroup this host belongs to, if any.",
                None,
            )
            .annotate(read_only()),
            |ctx, Empty {}| async move {
                let status = ctx.bridge().await?.status().await.map_err(Error::Bridge)?;
                json(&status.workgroup)
            },
        ))
        .with_route(route::<S, Empty, _, _>(
            tool(
                "device_list",
                "List the workgroup's devices: name, id, election priority and whether each \
                 is online.",
                None,
            )
            .annotate(read_only()),
            |ctx, Empty {}| async move {
                let peers = ctx.bridge().await?.peers().await.map_err(Error::Bridge)?;
                json(&peers.peers)
            },
        ))
}

/// The tools that write the workspace's files: `workspace_write_file`,
/// `workspace_append_file` and `workspace_delete_file`.
///
/// Not part of [`framework_tools`] — see the module docs.
pub fn workspace_write_tools<S>() -> ToolRouter<S>
where
    S: AsRef<ToolContext> + Send + Sync + 'static,
{
    ToolRouter::new()
        .with_route(route::<S, ContentArgs, _, _>(
            tool(
                "workspace_write_file",
                "Create or overwrite a text file of the workspace.",
                Some(schema_for_type::<ContentArgs>()),
            )
            .annotate(ToolAnnotations::new().read_only(false).destructive(true)),
            |ctx, args: ContentArgs| async move {
                let _: proto::Ack = ctx
                    .call(
                        proto::WRITE_FILE,
                        proto::ContentParams {
                            path: args.path.clone(),
                            content: args.content,
                        },
                    )
                    .await?;
                done(format!("wrote {}", args.path.display()))
            },
        ))
        .with_route(route::<S, ContentArgs, _, _>(
            tool(
                "workspace_append_file",
                "Append text to a file of the workspace, creating it if it does not exist.",
                Some(schema_for_type::<ContentArgs>()),
            )
            .annotate(ToolAnnotations::new().read_only(false).destructive(false)),
            |ctx, args: ContentArgs| async move {
                let _: proto::Ack = ctx
                    .call(
                        proto::APPEND_FILE,
                        proto::ContentParams {
                            path: args.path.clone(),
                            content: args.content,
                        },
                    )
                    .await?;
                done(format!("appended to {}", args.path.display()))
            },
        ))
        .with_route(route::<S, PathArgs, _, _>(
            tool(
                "workspace_delete_file",
                "Delete a file of the workspace.",
                Some(schema_for_type::<PathArgs>()),
            )
            .annotate(ToolAnnotations::new().read_only(false).destructive(true)),
            |ctx, args: PathArgs| async move {
                let _: proto::Ack = ctx
                    .call(
                        proto::DELETE_FILE,
                        proto::PathParams {
                            path: args.path.clone(),
                        },
                    )
                    .await?;
                done(format!("deleted {}", args.path.display()))
            },
        ))
}

/// A tool that takes no arguments.
#[derive(Debug, Deserialize, JsonSchema)]
struct Empty {}

/// `workspace_search`'s arguments.
#[derive(Debug, Deserialize, JsonSchema)]
struct SearchArgs {
    /// What to look for.
    query: String,
    /// The most files to return. Defaults to 10.
    #[serde(default)]
    limit: Option<usize>,
    /// `fts` (full text), `semantic` (vectors; full text when no embedder is configured)
    /// or `hybrid` (both, fused — the default).
    #[serde(default)]
    mode: Option<Mode>,
}

/// [`proto::SearchMode`], with a schema.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum Mode {
    Fts,
    Semantic,
    #[default]
    Hybrid,
}

impl From<Mode> for proto::SearchMode {
    fn from(mode: Mode) -> Self {
        match mode {
            Mode::Fts => proto::SearchMode::Fts,
            Mode::Semantic => proto::SearchMode::Semantic,
            Mode::Hybrid => proto::SearchMode::Hybrid,
        }
    }
}

/// Arguments naming a file.
#[derive(Debug, Deserialize, JsonSchema)]
struct PathArgs {
    /// The file's path, relative to the workspace root.
    path: PathBuf,
}

/// `workspace_list_dir`'s arguments.
#[derive(Debug, Deserialize, JsonSchema)]
struct ListDirArgs {
    /// The directory, relative to the workspace root. Defaults to the root.
    #[serde(default)]
    path: Option<PathBuf>,
}

/// Arguments naming a file and the text to put there.
#[derive(Debug, Deserialize, JsonSchema)]
struct ContentArgs {
    /// The file's path, relative to the workspace root.
    path: PathBuf,
    /// The text.
    content: String,
}

/// A tool's declaration; `schema` `None` for one that takes no arguments.
fn tool(
    name: &'static str,
    description: &'static str,
    schema: Option<std::sync::Arc<JsonObject>>,
) -> Tool {
    Tool::new(
        name,
        description,
        schema.unwrap_or_else(schema_for_empty_input),
    )
}

fn read_only() -> ToolAnnotations {
    ToolAnnotations::new().read_only(true)
}

/// A route whose handler takes the context and the parsed arguments.
///
/// A handler's [`Error`] becomes the tool's error result — shown to the agent, which can
/// act on it — rather than a protocol error: a server that is not running or a file that is
/// not there is an answer, not a broken session. Arguments that do not parse are the same.
fn route<S, A, F, Fut>(tool: Tool, handler: F) -> ToolRoute<S>
where
    S: AsRef<ToolContext> + Send + Sync + 'static,
    A: DeserializeOwned + Send + 'static,
    F: Fn(ToolContext, A) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<CallToolResult>> + Send + 'static,
{
    let handler = std::sync::Arc::new(handler);
    ToolRoute::new_dyn(tool, move |call: ToolCallContext<'_, S>| {
        let ctx = call.service.as_ref().clone();
        let arguments = call.arguments.clone().unwrap_or_default();
        let handler = std::sync::Arc::clone(&handler);
        Box::pin(async move {
            let outcome = match serde_json::from_value::<A>(serde_json::Value::Object(arguments)) {
                Ok(args) => handler(ctx, args).await,
                Err(err) => Err(Error::Arguments(err)),
            };
            Ok::<_, ErrorData>(outcome.unwrap_or_else(|err| {
                CallToolResult::error(vec![ContentBlock::text(err.to_string())])
            }))
        })
    })
}

/// A result carrying `value` as pretty-printed JSON.
fn json(value: &impl serde::Serialize) -> Result<CallToolResult> {
    Ok(CallToolResult::success(vec![ContentBlock::text(
        serde_json::to_string_pretty(value)?,
    )]))
}

/// A result carrying one line of text.
fn done(text: String) -> Result<CallToolResult> {
    Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Handler(ToolContext);

    impl AsRef<ToolContext> for Handler {
        fn as_ref(&self) -> &ToolContext {
            &self.0
        }
    }

    #[test]
    fn the_read_only_set_is_read_only_and_the_write_set_is_not() {
        let read = framework_tools::<Handler>().list_all();
        let names: Vec<_> = read.iter().map(|t| t.name.to_string()).collect();
        for expected in [
            "status",
            "workspace_show",
            "workspace_search",
            "workspace_read_file",
            "workspace_list_dir",
            "workgroup_show",
            "device_list",
        ] {
            assert!(
                names.iter().any(|n| n == expected),
                "missing {expected}: {names:?}"
            );
        }
        for tool in &read {
            let hint = tool.annotations.as_ref().and_then(|a| a.read_only_hint);
            assert_eq!(hint, Some(true), "{} is not marked read-only", tool.name);
        }

        let write = workspace_write_tools::<Handler>().list_all();
        assert_eq!(write.len(), 3);
        for tool in &write {
            let hint = tool.annotations.as_ref().and_then(|a| a.read_only_hint);
            assert_eq!(hint, Some(false), "{} is marked read-only", tool.name);
            assert!(
                !names.contains(&tool.name.to_string()),
                "{} is in both sets",
                tool.name
            );
        }
    }

    #[test]
    fn every_input_schema_is_an_object() {
        let tools = (framework_tools::<Handler>() + workspace_write_tools::<Handler>()).list_all();
        for tool in tools {
            assert_eq!(
                tool.input_schema.get("type").and_then(|t| t.as_str()),
                Some("object"),
                "{}'s input schema is not an object",
                tool.name
            );
        }
    }
}
