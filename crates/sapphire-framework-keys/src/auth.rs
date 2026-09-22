#![cfg(feature = "axum")]

//! 鍵による認証を tower layer として提供する。
//!
//! framework のルートと、アプリが同じプロセスで生やす自前のルート（MCP など）に
//! **同じ鍵**をかけられるようにするためのもの。`/rpc` は守られているのに `/mcp` は
//! 素通し、という事故を避ける。

use std::sync::Arc;

use axum::{
    Router,
    extract::{Request, State},
    http::StatusCode,
    middleware::{Next, from_fn_with_state},
    response::Response,
};
use uuid::Uuid;

use crate::AuthConfig;

/// 認証に成功したリクエストの拡張として入る値。
///
/// 将来 `Change` に書き込み元を持たせるときは、rpc 型に 1 フィールド足して
/// ここの `device_id` を読むだけでよい — コンテンツに焼き込む書き手の同一性は
/// 鍵の UUID（`key_id`）ではなく、鍵が指すデバイスの grain-id の方。`key_id` は
/// 鍵ファイル内部の同一性でしかなく、人間やドキュメントが参照する主体ではない。
#[derive(Clone, Debug)]
pub struct Authenticated {
    pub key_id: Uuid,
    /// アプリの `devices.toml` のエントリを指す。鍵に `device_id` が設定されて
    /// いない場合は `None`。
    pub device_id: Option<grain_id::GrainId>,
    pub label: Option<String>,
}

/// `router` を `guard` の鍵で保護して返す。
///
/// 鍵ストアが設定されていない場合は**閉じる**：全リクエストを 503 で拒否する
/// レイヤを被せる。素通しにはしない。
///
/// 「鍵が無ければ待ち受けない」保証を `serve` だけに置くのでは足りない — アプリ
/// が `/rpc` と `/mcp` を同じポートに載せる構成では、アプリは自前の listener を
/// 持つので `serve` を通らず
/// `axum::serve(listener, router(state).merge(protect(state, mcp)))` を呼ぶ。
/// 保証はレイヤ側に無ければ意味がない。
///
/// 401 ではなく 503 なのは、これが資格情報の問題ではなくサーバの設定漏れだから。
/// クライアントに再試行や鍵の入れ直しを促すべき状況ではない。
///
/// 意図して認証を外したい場合は
/// [`AuthConfig::insecure_for_tests`] を呼ぶ。逃げ道に名前を与えてあるので、
/// 設定漏れと区別がつく。
pub fn protect(guard: Arc<AuthConfig>, router: Router) -> Router {
    if guard.keys().is_none() {
        if guard.is_insecure() {
            tracing::warn!(
                "AuthConfig::insecure_for_tests() is set; this router is unauthenticated"
            );
            return router;
        }
        tracing::error!("no key store configured; this router will refuse every request");
        return router.layer(from_fn_with_state(guard, refuse));
    }
    router.layer(from_fn_with_state(guard, authenticate))
}

/// 鍵ストアが無いときに被せるレイヤ。何も通さない。
async fn refuse(
    State(_guard): State<Arc<AuthConfig>>,
    _request: Request,
    _next: Next,
) -> std::result::Result<Response, StatusCode> {
    Err(StatusCode::SERVICE_UNAVAILABLE)
}

async fn authenticate(
    State(guard): State<Arc<AuthConfig>>,
    mut request: Request,
    next: Next,
) -> std::result::Result<Response, StatusCode> {
    // `protect` はこのレイヤを鍵ストアがあるときにしか被せないので、ここは
    // 到達しない。到達したなら鍵ストアが実行中に消えたということ（将来の
    // ホットリロード等）なので、素通しではなく拒否する。
    let Some(keys) = guard.keys() else {
        tracing::error!("key store vanished while the auth layer was installed; refusing");
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    };

    let presented = request
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or(StatusCode::UNAUTHORIZED)?;

    let entry = keys
        .authenticate(presented)
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let who = Authenticated {
        key_id: entry.id,
        device_id: entry.device_id,
        label: entry.label.clone(),
    };

    request.extensions_mut().insert(who);
    Ok(next.run(request).await)
}

#[cfg(all(test, feature = "axum"))]
mod tests {
    use super::*;
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode, header};
    use http_body_util::BodyExt as _;
    use tower::ServiceExt as _;

    /// The four axum-level behaviours this middleware pins, moved from the
    /// remote server's `tests/rpc.rs` (the rest of that file stays with the
    /// router). They need more than a bare `KeyStore` — the fail-closed branch
    /// and the test-only bypass are layer configuration — so they run against
    /// an [`AuthConfig`] instead of the old `ServerState`.
    ///
    /// `guard(Some(token))` wraps a real key file; `guard(None)` configures no
    /// key store at all and flips on the test-only bypass.
    fn guard(token: Option<&str>) -> Arc<AuthConfig> {
        match token {
            // テストは固定トークンを使いたいので、生成ではなく直接書いた鍵を読ませる。
            Some(t) => {
                let tmp = tempfile::tempdir().unwrap();
                let key_path = tmp.path().join("keys.toml");
                std::fs::write(&key_path, format!("[[key]]\ntoken = \"{t}\"\n")).unwrap();
                Arc::new(AuthConfig::new(Arc::new(
                    crate::KeyStore::load(&key_path).unwrap(),
                )))
            }
            // 鍵ストアの無いルータは既定で全リクエストを 503 で拒否する。無認証で
            // 通したいなら明示的に言う。
            None => Arc::new(AuthConfig::unconfigured().insecure_for_tests()),
        }
    }

    #[tokio::test]
    async fn protect_guards_a_foreign_route_with_the_same_key() {
        let st = guard(Some("sjt_secret"));
        let app = protect(
            Arc::clone(&st),
            Router::new().route("/mcp", axum::routing::get(|| async { "ok" })),
        );

        let unauthorized = app
            .clone()
            .oneshot(Request::builder().uri("/mcp").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

        let authorized = app
            .oneshot(
                Request::builder()
                    .uri("/mcp")
                    .header(header::AUTHORIZATION, "Bearer sjt_secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(authorized.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn an_authenticated_request_carries_the_key_id() {
        let st = guard(Some("sjt_secret"));
        let key_id = st.keys().unwrap().entries()[0].id;

        let app = protect(
            Arc::clone(&st),
            Router::new().route(
                "/whoami",
                axum::routing::get(
                    |axum::Extension(who): axum::Extension<Authenticated>| async move {
                        who.key_id.to_string()
                    },
                ),
            ),
        );

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/whoami")
                    .header(header::AUTHORIZATION, "Bearer sjt_secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(
            String::from_utf8(body.to_vec()).unwrap(),
            key_id.to_string()
        );
    }

    #[tokio::test]
    async fn protect_without_a_key_store_refuses_the_app_s_own_routes() {
        // アプリが /rpc と /mcp を同じポートに載せるとき、アプリは自前の listener を
        // 持つので `serve` の「鍵が無ければ起動しない」検査を通らない。保証は
        // レイヤ側に無ければ意味がない。
        let st = Arc::new(AuthConfig::unconfigured());

        let app = protect(
            Arc::clone(&st),
            Router::new().route("/mcp", axum::routing::get(|| async { "ok" })),
        );

        let response = app
            .oneshot(Request::builder().uri("/mcp").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(
            response.status(),
            StatusCode::SERVICE_UNAVAILABLE,
            "鍵ストアが無いのに素通ししてはならない"
        );
    }

    #[tokio::test]
    async fn insecure_for_tests_is_the_only_way_through_without_keys() {
        let st = guard(None);
        assert!(st.is_insecure());

        let app = protect(
            Arc::clone(&st),
            Router::new().route("/mcp", axum::routing::get(|| async { "ok" })),
        );
        let response = app
            .oneshot(Request::builder().uri("/mcp").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }
}
