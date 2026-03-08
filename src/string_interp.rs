#[derive(Clone, Debug)]
pub enum Segment {
    Text(String),
    Variable(String),
}

pub fn parse_segments(input: &str) -> Result<Vec<Segment>, String> {
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0usize;
    let mut text = String::new();
    let mut segments = Vec::new();

    while i < chars.len() {
        if chars[i] == '\\' && i + 1 < chars.len() && chars[i + 1] == '(' {
            if !text.is_empty() {
                segments.push(Segment::Text(std::mem::take(&mut text)));
            }
            i += 2;

            if i >= chars.len() {
                return Err("Unclosed interpolation: expected ')'".to_string());
            }

            let start = i;
            if !is_ident_start(chars[i]) {
                return Err("Interpolation expects identifier: \\(name)".to_string());
            }
            i += 1;
            while i < chars.len() && is_ident_continue(chars[i]) {
                i += 1;
            }

            if i >= chars.len() || chars[i] != ')' {
                return Err("Unclosed interpolation: expected ')'".to_string());
            }

            let name: String = chars[start..i].iter().collect();
            segments.push(Segment::Variable(name));
            i += 1;
            continue;
        }

        text.push(chars[i]);
        i += 1;
    }

    if !text.is_empty() || segments.is_empty() {
        segments.push(Segment::Text(text));
    }

    Ok(segments)
}

fn is_ident_start(ch: char) -> bool {
    ch.is_ascii_alphabetic() || ch == '_'
}

fn is_ident_continue(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}
