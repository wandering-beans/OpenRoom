// Included in the development tests so the benchmark uses the real preview boundary.
#[test]
#[ignore = "requires AE_PREVIEW_PHOTO, AE_TEST_EXECUTABLE, and AE_BENCH_ADDRESS_FILE"]
fn frontend_benchmark_server() {
    use std::io::{BufRead, Read, Write};
    let (_directory, state, _) = fixture();
    let source = PathBuf::from(std::env::var_os("AE_PREVIEW_PHOTO").unwrap())
        .canonicalize()
        .unwrap();
    let mut catalog = Catalog::open(&state.db).unwrap();
    catalog.add_root(source.parent().unwrap()).unwrap();
    catalog.scan(&AtomicBool::new(false), |_| {}).unwrap();
    let id = catalog
        .page(
            &advanced_explorer::Query {
                text: source.file_name().unwrap().to_string_lossy().into_owned(),
                ..Default::default()
            },
            1,
            None,
        )
        .unwrap()
        .items[0]
        .id;
    assert_eq!(catalog.source_path(id).unwrap(), source);
    let develop = Arc::new(DevelopState::new().unwrap());
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    std::fs::write(
        std::env::var_os("AE_BENCH_ADDRESS_FILE").unwrap(),
        format!("http://{}", listener.local_addr().unwrap()),
    )
    .unwrap();
    for stream in listener.incoming() {
        let mut stream = stream.unwrap();
        let state = state.clone();
        let develop = develop.clone();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(120)))
            .unwrap();
        let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let stop = line.starts_with("POST /stop ");
        let save = line.starts_with("POST /save ");
        let state_request = line.starts_with("POST /get ");
        let mut length = 0;
        let mut header_bytes = line.len();
        loop {
            line.clear();
            reader.read_line(&mut line).unwrap();
            header_bytes += line.len();
            assert!(header_bytes <= 16384, "oversized benchmark HTTP headers");
            if line == "\r\n" {
                break;
            }
            if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                length = value.trim().parse::<usize>().unwrap();
            }
        }
        assert!(length <= 8 * 1024 * 1024);
        let mut body = vec![0; length];
        reader.read_exact(&mut body).unwrap();
        let handle = std::thread::spawn(move || {
            let result = if stop {
                serde_json::json!({"stopped":true})
            } else if state_request {
                let (recipe, revision) = Catalog::open(&state.db)
                    .unwrap()
                    .develop_recipe(id)
                    .unwrap();
                serde_json::json!({"settings": recipe.map(|json| serde_json::from_str::<Settings>(&json).unwrap()).unwrap_or_default(), "revision":revision,"defaults":Settings::default()})
            } else if save {
                #[derive(serde::Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Save {
                    settings: Settings,
                    expected_revision: i64,
                    steps: Vec<EditStep>,
                    undo: Option<Vec<Settings>>,
                }
                let input: Save = serde_json::from_slice(&body).unwrap();
                match save_inner(
                    &state,
                    id,
                    input.settings,
                    input.expected_revision,
                    input.steps,
                    input.undo,
                ) {
                    Ok(revision) => serde_json::json!(revision),
                    Err(error) => serde_json::json!({"error":error.to_string()}),
                }
            } else {
                #[derive(serde::Deserialize)]
                struct Input {
                    settings: Settings,
                    bound: u32,
                }
                let input: Input = serde_json::from_slice(&body).unwrap();
                let result = tauri::async_runtime::block_on(preview_inner(
                    &state,
                    &develop,
                    id,
                    input.settings,
                    input.bound,
                ));
                match result {
                    Ok(mut frame) => {
                        if frame.inline_image.is_none() {
                            frame.inline_image = Some(format!(
                                "data:image/png;base64,{}",
                                base64::engine::general_purpose::STANDARD
                                    .encode(std::fs::read(&frame.path).unwrap())
                            ));
                            frame.path.clear();
                        }
                        serde_json::to_value(frame).unwrap()
                    }
                    Err(error) => serde_json::json!({"error":error}),
                }
            };
            let bytes = serde_json::to_vec(&result).unwrap();
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", bytes.len()).unwrap();
            stream.write_all(&bytes).unwrap();
        });
        if stop {
            handle.join().unwrap();
            break;
        }
    }
}
