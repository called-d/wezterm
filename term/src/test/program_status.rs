//! OSC 7501 through the escape sequence parser and the terminal model
use super::*;
use k9::assert_equal as assert_eq;

#[derive(Clone, Default)]
struct SharedWriter(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for SharedWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn term_with_writer() -> (Terminal, SharedWriter) {
    let writer = SharedWriter::default();
    let term = Terminal::new(
        TerminalSize::default(),
        Arc::new(TestTermConfig { scrollback: 0 }),
        "WezTerm",
        "O_o",
        Box::new(writer.clone()),
    );
    (term, writer)
}

fn status(term: &Terminal) -> Option<String> {
    term.user_vars().get("WEZTERM_PROGRAM_STATUS").cloned()
}

#[test]
fn query_reply() {
    let (mut term, writer) = term_with_writer();
    term.advance_bytes("\x1b]7501;?\x07");
    assert_eq!(
        String::from_utf8(writer.0.lock().unwrap().clone()).unwrap(),
        "\x1b]7501;?:states=idle,working,done,blocked,error:kinds=permission,question,auth\x1b\\"
    );
}

#[test]
fn records_follow_reports_and_prompts() {
    let (mut term, _writer) = term_with_writer();
    assert_eq!(status(&term), None);

    term.advance_bytes("\x1b]7501;state=working:app=cargo:progress=10\x1b\\");
    term.advance_bytes("\x1b]7501;state=done:id=test\x1b\\");
    assert_eq!(
        status(&term).as_deref(),
        Some(
            r#"[{"app":"cargo","id":"","progress":10,"state":"working"},{"app":"cargo","id":"test","state":"done"}]"#
        )
    );

    // a malformed report leaves the records alone
    term.advance_bytes("\x1b]7501;state=bogus\x1b\\");
    // and the user var cannot be overwritten directly
    term.advance_bytes("\x1b]1337;SetUserVar=WEZTERM_PROGRAM_STATUS=W10=\x07");
    assert!(status(&term).unwrap().contains("working"));

    // OSC 133 A drops the working record and keeps the done one
    term.advance_bytes("\x1b]133;A\x1b\\");
    assert_eq!(
        status(&term).as_deref(),
        Some(r#"[{"id":"test","state":"done"}]"#)
    );

    // RIS clears everything
    term.advance_bytes("\x1bc");
    assert_eq!(status(&term).as_deref(), Some("[]"));
}
