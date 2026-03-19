//! E2E test: real github WASM tool with parameter coercion.
//!
//! Loads the actual compiled github WASM binary, sends params with string-typed
//! numbers (the exact bug scenario), and verifies the WASM tool constructs the
//! correct HTTP API call with coerced values. Uses an HTTP interceptor to capture
//! the request without hitting the real GitHub API.

#[cfg(feature = "libsql")]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Arc;

    use async_trait::async_trait;
    use serde_json::json;
    use tokio::sync::Mutex;

    use ironclaw::context::JobContext;
    use ironclaw::llm::recording::{
        HttpExchangeRequest, HttpExchangeResponse, HttpInterceptor,
    };
    use ironclaw::tools::prepare_tool_params;
    use ironclaw::tools::Tool;
    use ironclaw::tools::wasm::{
        Capabilities, EndpointPattern, HttpCapability, SecretsCapability,
        WasmRuntimeConfig, WasmToolRuntime, WasmToolWrapper,
    };

    /// Path to the pre-compiled github WASM tool binary.
    /// Built with: cargo build --manifest-path tools-src/github/Cargo.toml --target wasm32-wasip2 --release
    const GITHUB_WASM_PATH: &str = "tools-src/github/target/wasm32-wasip2/release/github_tool.wasm";

    /// HTTP interceptor that captures requests and returns canned responses.
    #[derive(Debug)]
    struct CapturingInterceptor {
        captured: Mutex<Vec<HttpExchangeRequest>>,
        responses: Mutex<VecDeque<HttpExchangeResponse>>,
    }

    impl CapturingInterceptor {
        fn new(responses: Vec<HttpExchangeResponse>) -> Self {
            Self {
                captured: Mutex::new(Vec::new()),
                responses: Mutex::new(VecDeque::from(responses)),
            }
        }

        async fn captured_requests(&self) -> Vec<HttpExchangeRequest> {
            self.captured.lock().await.clone()
        }
    }

    #[async_trait]
    impl HttpInterceptor for CapturingInterceptor {
        async fn before_request(
            &self,
            request: &HttpExchangeRequest,
        ) -> Option<HttpExchangeResponse> {
            self.captured.lock().await.push(request.clone());
            let mut queue = self.responses.lock().await;
            Some(queue.pop_front().unwrap_or_else(|| HttpExchangeResponse {
                status: 500,
                headers: vec![],
                body: r#"{"error": "no more canned responses"}"#.to_string(),
            }))
        }

        async fn after_response(
            &self,
            _request: &HttpExchangeRequest,
            _response: &HttpExchangeResponse,
        ) {
        }
    }

    fn github_capabilities() -> Capabilities {
        Capabilities {
            http: Some(HttpCapability {
                allowlist: vec![EndpointPattern {
                    host: "api.github.com".to_string(),
                    path_prefix: Some("/".to_string()),
                    methods: vec!["GET".to_string(), "POST".to_string(), "PUT".to_string()],
                }],
                ..Default::default()
            }),
            secrets: Some(SecretsCapability {
                allowed_names: vec!["github_token".to_string()],
            }),
            ..Default::default()
        }
    }

    fn github_api_response(body: &str) -> HttpExchangeResponse {
        HttpExchangeResponse {
            status: 200,
            headers: vec![
                ("content-type".to_string(), "application/json".to_string()),
                ("x-ratelimit-remaining".to_string(), "100".to_string()),
            ],
            body: body.to_string(),
        }
    }

    async fn load_github_tool(
        interceptor: Arc<dyn HttpInterceptor>,
    ) -> Option<WasmToolWrapper> {
        let wasm_path = std::path::Path::new(GITHUB_WASM_PATH);
        if !wasm_path.exists() {
            eprintln!(
                "Skipping WASM github test: binary not found at {GITHUB_WASM_PATH}. \
                 Build with: CARGO_TARGET_DIR=tools-src/github/target \
                 cargo build --manifest-path tools-src/github/Cargo.toml \
                 --target wasm32-wasip2 --release"
            );
            return None;
        }

        let wasm_bytes = std::fs::read(wasm_path).expect("read WASM binary");
        let runtime = Arc::new(
            WasmToolRuntime::new(WasmRuntimeConfig::default())
                .expect("create WASM runtime"),
        );
        let prepared = runtime
            .prepare("github", &wasm_bytes, None)
            .await
            .expect("prepare WASM module");

        let wrapper = WasmToolWrapper::new(runtime, prepared, github_capabilities())
            .with_http_interceptor(interceptor);

        Some(wrapper)
    }

    /// Reproduces the exact bug: LLM sends `limit: "50"` as a string to the
    /// `list_issues` action. The coercion layer must convert it to `50` (integer)
    /// before the WASM tool deserializes it, and the resulting HTTP request must
    /// contain `per_page=50` in the URL.
    #[tokio::test]
    async fn wasm_github_list_issues_coerces_limit_to_correct_url() {
        let interceptor = Arc::new(CapturingInterceptor::new(vec![github_api_response(
            r#"[{"number":1,"title":"Test issue","state":"open"}]"#,
        )]));

        let Some(tool) = load_github_tool(interceptor.clone()).await else {
            return; // WASM binary not available
        };

        // Simulate LLM sending string-typed numeric params
        let raw_params = json!({
            "action": "list_issues",
            "owner": "nearai",
            "repo": "ironclaw",
            "state": "open",
            "limit": "50"  // BUG: string instead of integer
        });

        // Run through the same coercion the agent loop applies
        let coerced = prepare_tool_params(&tool, &raw_params);

        // Execute the real WASM tool
        let ctx = JobContext::default();
        let result = tool.execute(coerced, &ctx).await;
        assert!(result.is_ok(), "Tool execution failed: {:?}", result.err());

        // Verify the captured HTTP request
        let requests = interceptor.captured_requests().await;
        assert_eq!(requests.len(), 1, "Expected exactly one HTTP request");
        assert_eq!(requests[0].method, "GET");
        assert!(
            requests[0].url.contains("/repos/nearai/ironclaw/issues"),
            "URL missing issues path: {}",
            requests[0].url
        );
        assert!(
            requests[0].url.contains("per_page=50"),
            "URL missing coerced per_page=50: {}",
            requests[0].url
        );
        assert!(
            requests[0].url.contains("state=open"),
            "URL missing state param: {}",
            requests[0].url
        );
    }

    /// Tests `get_issue` variant: `issue_number: "42"` must be coerced to
    /// integer 42 and appear in the URL as `/issues/42`.
    #[tokio::test]
    async fn wasm_github_get_issue_coerces_issue_number_to_correct_url() {
        let interceptor = Arc::new(CapturingInterceptor::new(vec![github_api_response(
            r#"{"number":42,"title":"Test","state":"open"}"#,
        )]));

        let Some(tool) = load_github_tool(interceptor.clone()).await else {
            return;
        };

        let raw_params = json!({
            "action": "get_issue",
            "owner": "nearai",
            "repo": "ironclaw",
            "issue_number": "42"  // BUG: string instead of integer
        });

        let coerced = prepare_tool_params(&tool, &raw_params);
        let ctx = JobContext::default();
        let result = tool.execute(coerced, &ctx).await;
        assert!(result.is_ok(), "Tool execution failed: {:?}", result.err());

        let requests = interceptor.captured_requests().await;
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, "GET");
        assert!(
            requests[0].url.contains("/repos/nearai/ironclaw/issues/42"),
            "URL missing /issues/42: {}",
            requests[0].url
        );
    }

    /// Tests `list_pull_requests` variant with coerced `limit`.
    #[tokio::test]
    async fn wasm_github_list_prs_coerces_limit_to_correct_url() {
        let interceptor = Arc::new(CapturingInterceptor::new(vec![github_api_response(
            r#"[{"number":1,"title":"Test PR","state":"open"}]"#,
        )]));

        let Some(tool) = load_github_tool(interceptor.clone()).await else {
            return;
        };

        let raw_params = json!({
            "action": "list_pull_requests",
            "owner": "nearai",
            "repo": "ironclaw",
            "limit": "25"  // BUG: string instead of integer
        });

        let coerced = prepare_tool_params(&tool, &raw_params);
        let ctx = JobContext::default();
        let result = tool.execute(coerced, &ctx).await;
        assert!(result.is_ok(), "Tool execution failed: {:?}", result.err());

        let requests = interceptor.captured_requests().await;
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, "GET");
        assert!(
            requests[0].url.contains("/repos/nearai/ironclaw/pulls"),
            "URL missing pulls path: {}",
            requests[0].url
        );
        assert!(
            requests[0].url.contains("per_page=25"),
            "URL missing coerced per_page=25: {}",
            requests[0].url
        );
    }
}
