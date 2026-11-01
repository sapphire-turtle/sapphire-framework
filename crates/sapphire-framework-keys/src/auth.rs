#![cfg(feature = "axum")]

//! The bearer-token check as a tower layer.
//!
//! The same layer guards the framework's routes and the routes an application adds in the
//! same process (`/mcp`, `/acp`, `/audio/ingest`), so one is never left open while another
//! is guarded.

use std::sync::Arc;

use axum::{
    Router,
    extract::{Request, State},
    http::StatusCode,
    middleware::{Next, from_fn_with_state},
    response::Response,
};

use crate::{AuthConfig, Verdict};

/// `router`, guarded by `guard`.
///
/// Without a verifier the layer fails **closed**: every request is refused with 401 — a
/// server whose authentication is not wired is misconfigured, not open. The named
/// [`AuthConfig::insecure_for_tests`] is the only way through without one.
///
/// A missing or refused token is 401; the scheme is matched in any case (RFC 7235). An
/// accepted request carries [`Authenticated`](crate::Authenticated) in its extensions.
pub fn protect(guard: AuthConfig, router: Router) -> Router {
    if guard.verifier().is_none() {
        if guard.is_insecure() {
            tracing::warn!(
                "AuthConfig::insecure_for_tests() is set; this router is unauthenticated"
            );
            return router;
        }
        tracing::error!("no verifier configured; this router will refuse every request");
        return router.layer(from_fn_with_state(Arc::new(guard), refuse));
    }
    router.layer(from_fn_with_state(Arc::new(guard), authenticate))
}

async fn refuse(
    State(_guard): State<Arc<AuthConfig>>,
    _request: Request,
    _next: Next,
) -> std::result::Result<Response, StatusCode> {
    Err(StatusCode::UNAUTHORIZED)
}

async fn authenticate(
    State(guard): State<Arc<AuthConfig>>,
    mut request: Request,
    next: Next,
) -> std::result::Result<Response, StatusCode> {
    let Some(verifier) = guard.verifier() else {
        return Err(StatusCode::UNAUTHORIZED);
    };
    let presented = request
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split_once(' '))
        .filter(|(scheme, token)| scheme.eq_ignore_ascii_case("bearer") && !token.is_empty())
        .map(|(_, token)| token)
        .ok_or(StatusCode::UNAUTHORIZED)?
        .to_owned();
    match verifier.verify(&presented).await {
        Verdict::Accepted(who) => {
            request.extensions_mut().insert(who);
            Ok(next.run(request).await)
        }
        Verdict::Forbidden => Err(StatusCode::FORBIDDEN),
        Verdict::Refused => Err(StatusCode::UNAUTHORIZED),
    }
}

/// `router`, refusing with 403 a request whose key may only read
/// ([`Access::Read`](crate::Access::Read)) unless its method is `GET`, `HEAD` or
/// `OPTIONS`. For the application's own routes, inside [`protect`]; `/mcp` checks each
/// tool instead, as every MCP request is a `POST`. A request without a key (only through
/// [`AuthConfig::insecure_for_tests`]) passes.
pub fn read_only_by_method(router: Router) -> Router {
    router.layer(axum::middleware::from_fn(by_method))
}

async fn by_method(request: Request, next: Next) -> std::result::Result<Response, StatusCode> {
    use axum::http::Method;
    let reads = matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS
    );
    let writer = request
        .extensions()
        .get::<crate::Authenticated>()
        .is_none_or(|who| who.access.may_write());
    if reads || writer {
        Ok(next.run(request).await)
    } else {
        Err(StatusCode::FORBIDDEN)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, header};
    use http_body_util::BodyExt as _;
    use tower::ServiceExt as _;

    use crate::{Access, Authenticated, KeyId, ProfileId, Verifier};

    /// Accepts `good` with `access`, forbids `"orphan"` (a profile the app dropped), refuses
    /// anything else.
    struct Fake {
        good: &'static str,
        id: KeyId,
        access: Access,
    }

    #[async_trait::async_trait]
    impl Verifier for Fake {
        async fn verify(&self, token: &str) -> Verdict {
            if token == self.good {
                Verdict::Accepted(Authenticated {
                    id: self.id.clone(),
                    name: "pendant".into(),
                    profile: ProfileId::new("p").unwrap(),
                    access: self.access,
                })
            } else if token == "orphan" {
                Verdict::Forbidden
            } else {
                Verdict::Refused
            }
        }
    }

    fn fake(good: &'static str, access: Access) -> AuthConfig {
        AuthConfig::new(Arc::new(Fake {
            good,
            id: KeyId::generate(),
            access,
        }))
    }

    fn app(guard: AuthConfig) -> Router {
        protect(
            guard,
            Router::new().route(
                "/whoami",
                axum::routing::get(
                    |axum::Extension(who): axum::Extension<Authenticated>| async move {
                        format!("{} {}", who.name, who.id)
                    },
                ),
            ),
        )
    }

    async fn status(app: Router, token: Option<&str>) -> (StatusCode, String) {
        status_with(app, token.map(|t| format!("Bearer {t}"))).await
    }

    async fn status_with(app: Router, authorization: Option<String>) -> (StatusCode, String) {
        let mut req = Request::builder().uri("/whoami");
        if let Some(a) = authorization {
            req = req.header(header::AUTHORIZATION, a);
        }
        let res = app.oneshot(req.body(Body::empty()).unwrap()).await.unwrap();
        let code = res.status();
        let body = res.into_body().collect().await.unwrap().to_bytes();
        (code, String::from_utf8(body.to_vec()).unwrap())
    }

    #[tokio::test]
    async fn the_layer_admits_the_right_token_and_names_its_device() {
        let id = KeyId::generate();
        let guard = AuthConfig::new(Arc::new(Fake {
            good: "sapphire-ak-good",
            id: id.clone(),
            access: Access::Write,
        }));
        assert_eq!(
            status(app(guard.clone()), None).await.0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            status(app(guard.clone()), Some("sapphire-ak-bad")).await.0,
            StatusCode::UNAUTHORIZED
        );
        let (code, body) = status(app(guard), Some("sapphire-ak-good")).await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(body, format!("pendant {id}"));
    }

    #[tokio::test]
    async fn the_scheme_is_case_insensitive() {
        let guard = fake("t", Access::Write);
        for scheme in ["bearer", "BEARER"] {
            let (code, _) = status_with(app(guard.clone()), Some(format!("{scheme} t"))).await;
            assert_eq!(code, StatusCode::OK, "{scheme}");
        }
    }

    #[tokio::test]
    async fn without_a_verifier_everything_is_refused_unless_insecure() {
        assert_eq!(
            status(app(AuthConfig::unconfigured()), Some("x")).await.0,
            StatusCode::UNAUTHORIZED
        );
        let open = protect(
            AuthConfig::unconfigured().insecure_for_tests(),
            Router::new().route("/mcp", axum::routing::get(|| async { "ok" })),
        );
        let res = open
            .oneshot(Request::builder().uri("/mcp").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn a_key_of_a_dropped_profile_is_forbidden() {
        let (code, _) = status(app(fake("t", Access::Write)), Some("orphan")).await;
        assert_eq!(code, StatusCode::FORBIDDEN);
    }

    async fn call(access: Access, method: &str) -> StatusCode {
        let routes = read_only_by_method(Router::new().route(
            "/x",
            axum::routing::get(|| async { "r" }).post(|| async { "w" }),
        ));
        let app = protect(fake("t", access), routes);
        let req = Request::builder()
            .method(method)
            .uri("/x")
            .header(header::AUTHORIZATION, "Bearer t")
            .body(Body::empty())
            .unwrap();
        app.oneshot(req).await.unwrap().status()
    }

    #[tokio::test]
    async fn a_read_only_key_may_only_get() {
        assert_eq!(call(Access::Read, "GET").await, StatusCode::OK);
        assert_eq!(call(Access::Read, "POST").await, StatusCode::FORBIDDEN);
        assert_eq!(call(Access::Write, "POST").await, StatusCode::OK);
        assert_eq!(call(Access::Admin, "POST").await, StatusCode::OK);
    }
}
