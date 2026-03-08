#[derive(Clone, Debug)]
pub enum Segment {
    Text(String),
    Expression(String),
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
            let mut nesting = 0usize;
            while i < chars.len() {
                match chars[i] {
                    '(' => {
                        nesting += 1;
                        i += 1;
                    }
                    ')' => {
                        if nesting == 0 {
                            break;
                        }
                        nesting -= 1;
                        i += 1;
                    }
                    _ => i += 1,
                }
            }

            if i >= chars.len() || chars[i] != ')' {
                return Err("Unclosed interpolation: expected ')'".to_string());
            }

            let expr: String = chars[start..i].iter().collect();
            if expr.trim().is_empty() {
                return Err("Interpolation expression cannot be empty".to_string());
            }
            segments.push(Segment::Expression(expr));
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
