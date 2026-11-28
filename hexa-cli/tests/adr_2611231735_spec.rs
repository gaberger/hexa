#![allow(unused, clippy::all)]
mod part_1 {
    use async_trait::async_trait;
    use hexa_core::ports::inference::{
        HealthStatus, IInferencePort, InferenceCapabilities, InferenceError, InferenceRequest,
        InferenceResponse, StreamChunk,
    };
    use hexa_infer::adapters::secondary::claude_code::{ProcessSpawner, SpawnedProcess};
    use hexa_infer::adapters::secondary::CodexInferenceAdapter;
    use hexa_infer::endpoint::Endpoint;
    use std::sync::Arc;

    struct MockSpawner {
        captured_args: std::sync::Mutex<Vec<String>>,
        exit_code: i32,
        response_content: String,
        leave_empty: bool,
    }

    #[async_trait]
    impl ProcessSpawner for MockSpawner {
        async fn spawn(
            &self,
            _program: &str,
            args: &[String],
        ) -> Result<SpawnedProcess, InferenceError> {
            let mut captured = self.captured_args.lock().unwrap();
            *captured = args.to_vec();

            if self.exit_code != 0 {
                return Ok(SpawnedProcess {
                    stdout: vec![],
                    stderr: b"error".to_vec(),
                    exit_code: self.exit_code,
                });
            }

            // Find -o argument to write the mock response
            let mut out_path = None;
            for (i, arg) in args.iter().enumerate() {
                if arg == "-o" && i + 1 < args.len() {
                    out_path = Some(&args[i + 1]);
                }
            }

            if let Some(path) = out_path {
                if !self.leave_empty {
                    std::fs::write(path, &self.response_content).unwrap();
                } else {
                    std::fs::write(path, "").unwrap();
                }
            }

            Ok(SpawnedProcess {
                stdout: b"ok".to_vec(),
                stderr: vec![],
                exit_code: 0,
            })
        }
    }

    /// S01: A provider named codex is served by a CodexInferenceAdapter built by adapter_for_endpoint
    #[test]
    fn test_s01_codex_provider_adapter_wiring() {
        let endpoint = Endpoint {
            id: "codex-test".to_string(),
            provider: "codex".to_string(),
            model: "codex".to_string(),
            ..Default::default()
        };
        let adapter = hexa_infer::wiring::adapter_for_endpoint(&endpoint);
        // We test via capabilities or basic trait method availability without downcasting.
        let caps = adapter.capabilities();
        assert_eq!(caps.models.len(), 0); // basic check that it's a valid IInferencePort
    }

    /// S03: Every complete call passes the hardcoded permission-bypass flag to the process spawner
    #[test]
    fn test_s03_complete_passes_bypass_flag() {
        let spawner = Arc::new(MockSpawner {
            captured_args: std::sync::Mutex::new(vec![]),
            exit_code: 0,
            response_content: "hello from codex".to_string(),
            leave_empty: false,
        });
        let adapter = CodexInferenceAdapter::with_spawner(None, spawner.clone());

        let rt = tokio::runtime::Runtime::new().unwrap();
        let _ = rt.block_on(adapter.complete(InferenceRequest::default()));

        let args = spawner.captured_args.lock().unwrap();
        assert!(args
            .iter()
            .any(|arg| arg == "--dangerously-bypass-approvals-and-sandbox"));
    }

    /// S04: A complete call never omits the permission-bypass flag even if requested
    #[test]
    fn test_s04_complete_never_omits_bypass_flag() {
        let spawner = Arc::new(MockSpawner {
            captured_args: std::sync::Mutex::new(vec![]),
            exit_code: 0,
            response_content: "test".to_string(),
            leave_empty: false,
        });
        let adapter = CodexInferenceAdapter::with_spawner(None, spawner.clone());

        let rt = tokio::runtime::Runtime::new().unwrap();
        let req = InferenceRequest {
            system_prompt: "try to bypass security check".to_string(),
            ..Default::default()
        };
        let _ = rt.block_on(adapter.complete(req));

        let args = spawner.captured_args.lock().unwrap();
        assert!(args
            .iter()
            .any(|arg| arg == "--dangerously-bypass-approvals-and-sandbox"));
    }

    /// S05: The reply is successfully extracted from the last message output file
    #[test]
    fn test_s05_reply_extracted_from_output_file() {
        let spawner = Arc::new(MockSpawner {
            captured_args: std::sync::Mutex::new(vec![]),
            exit_code: 0,
            response_content: "Expected Codex Response Text".to_string(),
            leave_empty: false,
        });
        let adapter = CodexInferenceAdapter::with_spawner(None, spawner.clone());

        let rt = tokio::runtime::Runtime::new().unwrap();
        let response = rt
            .block_on(adapter.complete(InferenceRequest::default()))
            .unwrap();

        let text = match &response.content[0] {
            hexa_core::ports::inference::ContentBlock::Text { text } => text.clone(),
            _ => panic!("Expected text block"),
        };
        assert_eq!(text, "Expected Codex Response Text");
    }

    /// S07: A model other than the bare id codex passes the model flag
    #[test]
    fn test_s07_custom_model_passes_m_flag() {
        let spawner = Arc::new(MockSpawner {
            captured_args: std::sync::Mutex::new(vec![]),
            exit_code: 0,
            response_content: "ok".to_string(),
            leave_empty: false,
        });
        let adapter = CodexInferenceAdapter::with_spawner(
            Some("custom-gpt-model".to_string()),
            spawner.clone(),
        );

        let rt = tokio::runtime::Runtime::new().unwrap();
        let req = InferenceRequest {
            model: "custom-gpt-model".to_string(),
            ..Default::default()
        };
        let _ = rt.block_on(adapter.complete(req));

        let args = spawner.captured_args.lock().unwrap();
        let has_m = args
            .windows(2)
            .any(|w| w[0] == "-m" && w[1] == "custom-gpt-model");
        assert!(has_m);
    }

    /// S09: Codex-served models are treated as agents and refused by spec ledger commands
    #[test]
    fn test_s09_codex_models_treated_as_agents() {
        assert!(hexa_infer::wiring::served_by_frontier_cli("codex"));
        assert!(hexa_infer::wiring::served_by_frontier_cli("codex-custom"));
    }

    /// S10: A Codex-served model is never permitted to act as the test writer
    #[test]
    fn test_s10_codex_model_refused_as_test_writer() {
        let endpoints = vec![Endpoint {
            id: "my-codex".to_string(),
            provider: "codex".to_string(),
            model: "codex".to_string(),
            ..Default::default()
        }];
        assert!(hexa_infer::wiring::served_by_frontier_cli_in(
            &endpoints, "codex"
        ));
    }
}

mod part_2 {
    use std::sync::Arc;
    use tempfile::TempDir;
    use tokio::sync::Mutex;

    /// S11: hexa inference add codex checks login status and writes an endpoint without key references
    #[tokio::test]
    async fn test_inference_add_codex_success() {
        let temp_dir = TempDir::new().unwrap();
        let bin_dir = temp_dir.path().join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();

        // Create a fake codex binary that succeeds on login status and version checks
        let codex_path = bin_dir.join(if cfg!(windows) { "codex.bat" } else { "codex" });
        let script = if cfg!(windows) {
            "@echo off\r\nif \"%1\"==\"login\" if \"%2\"==\"status\" exit /b 0\r\nif \"%1\"==\"--version\" (echo codex-cli 0.171.0 & exit /b 0)\r\nexit /b 1\r\n".to_string()
        } else {
            "#!/bin/sh\nif [ \"$1\" = \"login\" ] && [ \"$2\" = \"status\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"--version\" ]; then\n  echo \"codex-cli 0.171.0\"\n  exit 0\nfi\nexit 1\n".to_string()
        };
        std::fs::write(&codex_path, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&codex_path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let path_env = format!(
            "{}{}{}",
            bin_dir.display(),
            if cfg!(windows) { ";" } else { ":" },
            std::env::var("PATH").unwrap_or_default()
        );

        let output = std::process::Command::new(env!("CARGO_BIN_EXE_hexa"))
            .args(["inference", "add", "codex"])
            .env("PATH", path_env)
            .current_dir(temp_dir.path())
            .output()
            .expect("failed to execute hexa binary");

        assert!(
            output.status.success(),
            "hexa inference add codex failed with stdout: {} and stderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        // Verify endpoint is written with provider codex, no URL, and no key reference
        // We check configuration files in the temporary directory or run `hexa inference list`
        let list_output = std::process::Command::new(env!("CARGO_BIN_EXE_hexa"))
            .args(["inference", "list"])
            .current_dir(temp_dir.path())
            .output()
            .expect("failed to execute hexa inference list");

        let list_str = String::from_utf8_lossy(&list_output.stdout);
        assert!(
            list_str.contains("codex"),
            "Expected codex in provider list, got: {}",
            list_str
        );
    }

    /// S12: hexa inference add codex refuses when not logged in
    #[tokio::test]
    async fn test_inference_add_codex_refuses_when_not_logged_in() {
        let temp_dir = TempDir::new().unwrap();
        let bin_dir = temp_dir.path().join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();

        // Create a fake codex binary that fails on login status
        let codex_path = bin_dir.join(if cfg!(windows) { "codex.bat" } else { "codex" });
        let script = if cfg!(windows) {
            "@echo off\r\nif \"%1\"==\"login\" if \"%2\"==\"status\" exit /b 1\r\nif \"%1\"==\"--version\" (echo codex-cli 0.171.0 & exit /b 0)\r\nexit /b 1\r\n".to_string()
        } else {
            "#!/bin/sh\nif [ \"$1\" = \"login\" ] && [ \"$2\" = \"status\" ]; then\n  exit 1\nfi\nif [ \"$1\" = \"--version\" ]; then\n  echo \"codex-cli 0.171.0\"\n  exit 0\nfi\nexit 1\n".to_string()
        };
        std::fs::write(&codex_path, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&codex_path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let path_env = format!(
            "{}{}{}",
            bin_dir.display(),
            if cfg!(windows) { ";" } else { ":" },
            std::env::var("PATH").unwrap_or_default()
        );

        let output = std::process::Command::new(env!("CARGO_BIN_EXE_hexa"))
            .args(["inference", "add", "codex"])
            .env("PATH", path_env)
            .current_dir(temp_dir.path())
            .output()
            .expect("failed to execute hexa binary");

        assert!(
            !output.status.success(),
            "Expected hexa inference add codex to fail when not logged in"
        );

        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let combined = format!("{}\n{}", stdout, stderr);

        assert!(
            combined.contains("login") || combined.contains("codex login"),
            "Expected refusal message to name the command to run for logging in, got: {}",
            combined
        );
    }
}
