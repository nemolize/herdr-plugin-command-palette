//! Asks a local Ollama for a workspace name: the request it is sent, the call,
//! and how its answer becomes a name.
use std::io::ErrorKind;
use std::net::ToSocketAddrs;
use std::time::Duration;

use serde_json::{json, Value};
use unicode_segmentation::UnicodeSegmentation;

use crate::settings::AutoName;

/// Grapheme clusters, not bytes or chars: a name is cut where a reader sees a
/// character end, so a CJK name keeps as many characters as a Latin one.
pub const NAME_CAP: usize = 20;

const SYSTEM_PROMPT: &str = "You name terminal workspaces. Read the panes below and reply with \
one label of at most 20 characters for what this workspace is about. Prefer the concrete \
subject (a project, feature, or task) over generic words such as terminal, dev, or work. \
Write the label in the language the human uses in the input. Reply with the label only: one \
line, no quotes, no explanation.";

/// `think: false` because Qwen3.5 thinks by default and is far slower with it
/// on. No `format`: the Homebrew build answers HTTP 501 to structured output
/// with MLX models, so the answer is plain text and `name_from` cleans it.
pub fn request_body(model: &str, prompt: &str) -> Value {
    json!({
        "model": model,
        "system": SYSTEM_PROMPT,
        "prompt": prompt,
        "stream": false,
        "think": false,
        "options": {"temperature": 0, "num_ctx": 8192},
        "keep_alive": "10m",
    })
}

/// The first non-empty line of `answer` without its control characters, its
/// surrounding whitespace and quotes stripped, cut to `NAME_CAP` — models
/// sometimes ignore the cap they are given. None when nothing is left.
pub fn name_from(answer: &str) -> Option<String> {
    let line = answer
        .lines()
        .map(|l| l.chars().filter(|c| !c.is_control()).collect::<String>())
        .find(|l| !l.trim().is_empty())?;
    let name = line.trim().trim_matches(is_quote).trim();
    let cut: String = name.graphemes(true).take(NAME_CAP).collect();
    let cut = cut.trim_end();
    (!cut.is_empty()).then(|| cut.to_string())
}

/// ASCII quotes, the curly ones, and CJK corner brackets. The curly ones are
/// matched by code point: they never reach the screen, and as literals the
/// glyph-width guard would read them as drawn text.
fn is_quote(c: char) -> bool {
    matches!(c, '"' | '\'' | '`') || matches!(u32::from(c), 0x2018..=0x201F | 0x300C..=0x300F)
}

/// The error is drawn on one row beside `Retry`, so it is kept short.
pub fn generate(settings: &AutoName, prompt: &str) -> Result<String, String> {
    if !resolves(&settings.base_url) {
        return Err(no_ollama_at(settings));
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(settings.timeout_secs)))
        // A 4xx/5xx carries Ollama's own reason in its body, which beats a bare
        // status code — so the body is read rather than turned into an error.
        .http_status_as_error(false)
        // ureq otherwise routes by HTTP_PROXY, which would send the panes'
        // output through a proxy that has no business seeing a local call.
        .proxy(None)
        .build()
        .into();
    let url = format!("{}/api/generate", settings.base_url.trim_end_matches('/'));
    let body = request_body(&settings.model, prompt).to_string();
    let mut response = agent
        .post(&url)
        .header("Content-Type", "application/json")
        .send(body)
        .map_err(|e| failure(&e, settings))?;
    let status = response.status().as_u16();
    let text = response
        .body_mut()
        .read_to_string()
        .map_err(|e| failure(&e, settings))?;
    answer_from(status, &text)
}

fn answer_from(status: u16, text: &str) -> Result<String, String> {
    let parsed: Option<Value> = serde_json::from_str(text).ok();
    if !(200..300).contains(&status) {
        let reason = parsed
            .as_ref()
            .and_then(|v| v.get("error"))
            .and_then(Value::as_str);
        return Err(match reason {
            Some(reason) => format!("HTTP {status}: {reason}"),
            None => format!("HTTP {status}"),
        });
    }
    let answer = parsed
        .as_ref()
        .and_then(|v| v.get("response"))
        .and_then(Value::as_str)
        .ok_or_else(|| "unreadable answer".to_string())?;
    name_from(answer).ok_or_else(|| "empty answer".to_string())
}

fn failure(e: &ureq::Error, settings: &AutoName) -> String {
    match e {
        ureq::Error::Timeout(_) => format!("timed out after {}s", settings.timeout_secs),
        ureq::Error::ConnectionFailed | ureq::Error::HostNotFound => no_ollama_at(settings),
        ureq::Error::Io(io) if NOT_LISTENING.contains(&io.kind()) => no_ollama_at(settings),
        other => other.to_string(),
    }
}

/// How a connect fails when nothing answers at the address; any other I/O error
/// came from a server that was there.
const NOT_LISTENING: [ErrorKind; 4] = [
    ErrorKind::ConnectionRefused,
    ErrorKind::HostUnreachable,
    ErrorKind::NetworkUnreachable,
    ErrorKind::AddrNotAvailable,
];

/// ureq reports a failed lookup as an uncategorised I/O error, which cannot be
/// told from a dropped connection afterwards — so the host is resolved first.
fn resolves(base_url: &str) -> bool {
    let authority = base_url
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap_or_default();
    let has_port = authority
        .rsplit_once(':')
        .is_some_and(|(_, p)| !p.contains(']'));
    let address = if has_port {
        authority.to_string()
    } else {
        format!("{authority}:80")
    };
    address.to_socket_addrs().is_ok()
}

fn no_ollama_at(settings: &AutoName) -> String {
    let host = settings.base_url.trim_start_matches("http://");
    format!("no Ollama at {}", host.trim_end_matches('/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_request_asks_for_one_unthought_non_streamed_answer() {
        let body = request_body("qwen3.5:9b", "panes");
        assert_eq!(
            body,
            json!({
                "model": "qwen3.5:9b",
                "system": SYSTEM_PROMPT,
                "prompt": "panes",
                "stream": false,
                "think": false,
                "options": {"temperature": 0, "num_ctx": 8192},
                "keep_alive": "10m",
            })
        );
        assert!(body.get("format").is_none(), "MLX models answer 501 to it");
    }

    #[test]
    fn the_name_is_the_first_non_empty_line() {
        assert_eq!(
            name_from("\n  \nrelease notes\nbecause the panes...").as_deref(),
            Some("release notes")
        );
    }

    #[test]
    fn surrounding_quotes_and_whitespace_are_stripped() {
        for answer in [
            "\"palette\"",
            "  'palette'  ",
            "`palette`",
            "\u{201C}palette\u{201D}",
            "\u{300C}palette\u{300D}",
            "\" palette \"",
        ] {
            assert_eq!(name_from(answer).as_deref(), Some("palette"), "{answer:?}");
        }
    }

    #[test]
    fn a_long_answer_is_cut_to_twenty_grapheme_clusters() {
        let name = name_from("abcdefghijklmnopqrstuvwxyz").unwrap();
        assert_eq!(name, "abcdefghijklmnopqrst");

        // Each ガ here is two chars (カ + combining mark) but one cluster.
        let wide = "\u{30AB}\u{3099}".repeat(25);
        let name = name_from(&wide).unwrap();
        assert_eq!(name.graphemes(true).count(), NAME_CAP);
        assert_eq!(name.chars().count(), 2 * NAME_CAP, "no mark split off");
    }

    #[test]
    fn a_cut_that_ends_in_a_space_does_not_keep_it() {
        assert_eq!(
            name_from("nineteen characters and more").as_deref(),
            Some("nineteen characters")
        );
    }

    #[test]
    fn control_characters_never_reach_the_name() {
        for (answer, name) in [
            ("\x1b[31mred\x1b[0m", "[31mred[0m"),
            ("tab\there", "tabhere"),
            ("carriage\rreturn", "carriagereturn"),
            ("c1\u{9b}x", "c1x"),
            ("\u{7}\"bell\"\u{7}", "bell"),
        ] {
            assert_eq!(name_from(answer).as_deref(), Some(name), "{answer:?}");
        }
    }

    #[test]
    fn a_line_of_control_characters_alone_is_skipped() {
        assert_eq!(name_from("\u{7}\n\x1b\nreal").as_deref(), Some("real"));
    }

    #[test]
    fn an_answer_with_nothing_left_is_no_name() {
        for answer in ["", "  \n\t\n", "\"\"", " '' "] {
            assert_eq!(name_from(answer), None, "{answer:?}");
        }
    }

    #[test]
    fn a_successful_response_yields_the_cleaned_name() {
        let body = r#"{"model":"m","response":"\"herdr palette\"\n","done":true}"#;
        assert_eq!(answer_from(200, body), Ok("herdr palette".to_string()));
    }

    #[test]
    fn an_empty_answer_is_a_failure_that_says_so() {
        let body = r#"{"response":"   ","done":true}"#;
        assert_eq!(answer_from(200, body), Err("empty answer".to_string()));
    }

    #[test]
    fn an_http_error_carries_ollamas_own_reason() {
        let body = r#"{"error":"model 'nope' not found"}"#;
        assert_eq!(
            answer_from(404, body),
            Err("HTTP 404: model 'nope' not found".to_string())
        );
        assert_eq!(answer_from(501, "not json"), Err("HTTP 501".to_string()));
    }

    #[test]
    fn a_body_without_a_response_is_unreadable() {
        assert_eq!(answer_from(200, "{}"), Err("unreadable answer".to_string()));
    }

    fn settings_at(base_url: &str, timeout_secs: u64) -> AutoName {
        AutoName {
            base_url: base_url.into(),
            timeout_secs,
            ..AutoName::default()
        }
    }

    /// Port 9 (discard) on loopback has nothing listening on any machine this
    /// runs on, so the connection is refused at once.
    #[test]
    fn an_unreachable_server_is_named_with_its_url() {
        let err = generate(&settings_at("http://127.0.0.1:9", 5), "x").unwrap_err();
        assert_eq!(err, "no Ollama at 127.0.0.1:9");
    }

    /// `.invalid` never resolves (RFC 2606), so this is a lookup failure.
    #[test]
    fn a_host_that_does_not_resolve_is_named() {
        let err = generate(&settings_at("http://no-such-host.invalid:11434", 5), "x").unwrap_err();
        assert_eq!(err, "no Ollama at no-such-host.invalid:11434");
    }

    #[test]
    fn a_base_url_resolves_with_or_without_its_port() {
        for url in [
            "http://127.0.0.1:11434",
            "http://localhost",
            "http://[::1]",
            "http://[::1]:11434/",
        ] {
            assert!(resolves(url), "{url}");
        }
    }

    /// A server that is there but drops the connection is not "no Ollama".
    #[test]
    fn a_connection_dropped_mid_request_is_not_reported_as_absent() {
        use std::io::Read;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = stream.read(&mut [0; 1024]);
        });
        let err = generate(&settings_at(&format!("http://127.0.0.1:{port}"), 5), "x").unwrap_err();
        assert!(!err.starts_with("no Ollama"), "{err}");
    }

    /// A listener that accepts and never answers stands in for a hung model.
    #[test]
    fn a_server_that_never_answers_times_out() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let _held = std::thread::spawn(move || {
            let held = listener.accept();
            std::thread::sleep(Duration::from_secs(5));
            drop(held);
        });
        let err = generate(&settings_at(&format!("http://127.0.0.1:{port}"), 1), "x").unwrap_err();
        assert_eq!(err, "timed out after 1s");
    }

    #[test]
    fn the_call_posts_the_body_and_reads_the_name() {
        use std::io::{BufRead, BufReader, Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut head = Vec::new();
            let mut length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = v.trim().parse().unwrap();
                }
                if line == "\r\n" {
                    break;
                }
                head.push(line);
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            let reply = r#"{"response":"palette work","done":true}"#;
            write!(
                &stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{reply}",
                reply.len()
            )
            .unwrap();
            (head, String::from_utf8(body).unwrap())
        });

        let settings = AutoName {
            model: "m".into(),
            ..settings_at(&format!("http://127.0.0.1:{port}/"), 5)
        };
        assert_eq!(
            generate(&settings, "the panes"),
            Ok("palette work".to_string())
        );
        let (head, body) = server.join().unwrap();
        assert!(head[0].starts_with("POST /api/generate "), "{:?}", head[0]);
        let sent: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(sent, request_body("m", "the panes"));
    }
}
