use std::io::{self, BufRead, Write};

pub(super) fn prompt_for_update(output: &mut impl Write) -> io::Result<bool> {
    let mut input = io::stdin().lock();
    loop {
        write!(output, "\nUpdate now? [Y/n] ")?;
        output.flush()?;

        let mut answer = String::new();
        if input.read_line(&mut answer)? == 0 {
            return Ok(false);
        }
        match answer.trim().to_ascii_lowercase().as_str() {
            "" | "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => writeln!(output, "Please answer yes or no.")?,
        }
    }
}
