//! Legacy render-only JSON-lines entry point, backed by KIT media controls.
use anyhow::{ensure, Result};
use starkit::media_controls::{render, Request};
use std::io::{BufRead, Read, Write};

pub fn run() -> Result<()> {
    let mut input = std::io::stdin().lock();
    let mut output = std::io::BufWriter::new(std::io::stdout().lock());
    loop {
        let mut line = Vec::new();
        if input.by_ref().take(65537).read_until(b'\n', &mut line)? == 0 {
            break;
        }
        ensure!(
            line.len() <= 65536 && line.last() == Some(&b'\n'),
            "Transport request exceeds limit"
        );
        let response = render(&serde_json::from_slice::<Request>(&line)?)?;
        serde_json::to_writer(&mut output, &response)?;
        output.write_all(b"\n")?;
        output.flush()?;
    }
    Ok(())
}
