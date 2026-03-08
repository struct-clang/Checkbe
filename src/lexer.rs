use crate::diagnostics::Diagnostics;
use crate::span::Span;
use crate::token::{Token, TokenKind};

pub fn lex(source: &str, diagnostics: &mut Diagnostics) -> Vec<Token> {
    let mut lexer = Lexer::new(source, diagnostics);
    lexer.lex_all()
}

struct Lexer<'a, 'b> {
    chars: Vec<char>,
    index: usize,
    line: usize,
    column: usize,
    diagnostics: &'b mut Diagnostics,
    _source: &'a str,
}

impl<'a, 'b> Lexer<'a, 'b> {
    fn new(source: &'a str, diagnostics: &'b mut Diagnostics) -> Self {
        Self {
            chars: source.chars().collect(),
            index: 0,
            line: 1,
            column: 1,
            diagnostics,
            _source: source,
        }
    }

    fn lex_all(&mut self) -> Vec<Token> {
        let mut tokens = Vec::new();

        loop {
            self.skip_ignored();
            let span = Span::new(self.line, self.column);

            let token = match self.current() {
                Some(ch) if is_ident_start(ch) => self.lex_identifier(span),
                Some(ch) if ch.is_ascii_digit() => self.lex_number(span),
                Some('"') => self.lex_string(span),
                Some('(') => {
                    self.bump();
                    Token {
                        kind: TokenKind::LParen,
                        span,
                    }
                }
                Some(')') => {
                    self.bump();
                    Token {
                        kind: TokenKind::RParen,
                        span,
                    }
                }
                Some('{') => {
                    self.bump();
                    Token {
                        kind: TokenKind::LBrace,
                        span,
                    }
                }
                Some('}') => {
                    self.bump();
                    Token {
                        kind: TokenKind::RBrace,
                        span,
                    }
                }
                Some('[') => {
                    self.bump();
                    Token {
                        kind: TokenKind::LBracket,
                        span,
                    }
                }
                Some(']') => {
                    self.bump();
                    Token {
                        kind: TokenKind::RBracket,
                        span,
                    }
                }
                Some(':') => {
                    self.bump();
                    Token {
                        kind: TokenKind::Colon,
                        span,
                    }
                }
                Some(';') => {
                    self.bump();
                    Token {
                        kind: TokenKind::Semicolon,
                        span,
                    }
                }
                Some(',') => {
                    self.bump();
                    Token {
                        kind: TokenKind::Comma,
                        span,
                    }
                }
                Some('.') => {
                    self.bump();
                    Token {
                        kind: TokenKind::Dot,
                        span,
                    }
                }
                Some('?') => {
                    self.bump();
                    Token {
                        kind: TokenKind::Question,
                        span,
                    }
                }
                Some('+') => {
                    self.bump();
                    Token {
                        kind: TokenKind::Plus,
                        span,
                    }
                }
                Some('-') => {
                    self.bump();
                    if self.current() == Some('>') {
                        self.bump();
                        Token {
                            kind: TokenKind::Arrow,
                            span,
                        }
                    } else {
                        Token {
                            kind: TokenKind::Minus,
                            span,
                        }
                    }
                }
                Some('*') => {
                    self.bump();
                    Token {
                        kind: TokenKind::Star,
                        span,
                    }
                }
                Some('%') => {
                    self.bump();
                    Token {
                        kind: TokenKind::Percent,
                        span,
                    }
                }
                Some('/') => {
                    self.bump();
                    Token {
                        kind: TokenKind::Slash,
                        span,
                    }
                }
                Some('=') => {
                    self.bump();
                    if self.current() == Some('=') {
                        self.bump();
                        Token {
                            kind: TokenKind::EqEq,
                            span,
                        }
                    } else {
                        Token {
                            kind: TokenKind::Assign,
                            span,
                        }
                    }
                }
                Some('!') => {
                    self.bump();
                    if self.current() == Some('=') {
                        self.bump();
                        Token {
                            kind: TokenKind::NotEq,
                            span,
                        }
                    } else {
                        Token {
                            kind: TokenKind::Bang,
                            span,
                        }
                    }
                }
                Some('>') => {
                    self.bump();
                    if self.current() == Some('=') {
                        self.bump();
                        Token {
                            kind: TokenKind::GreaterEq,
                            span,
                        }
                    } else {
                        Token {
                            kind: TokenKind::Greater,
                            span,
                        }
                    }
                }
                Some('<') => {
                    self.bump();
                    if self.current() == Some('=') {
                        self.bump();
                        Token {
                            kind: TokenKind::LessEq,
                            span,
                        }
                    } else {
                        Token {
                            kind: TokenKind::Less,
                            span,
                        }
                    }
                }
                Some('&') => {
                    self.bump();
                    if self.current() == Some('&') {
                        self.bump();
                        Token {
                            kind: TokenKind::AndAnd,
                            span,
                        }
                    } else {
                        self.diagnostics
                            .error(Some(span), "Expected '&' for '&&' operator");
                        continue;
                    }
                }
                Some('|') => {
                    self.bump();
                    if self.current() == Some('|') {
                        self.bump();
                        Token {
                            kind: TokenKind::OrOr,
                            span,
                        }
                    } else {
                        self.diagnostics
                            .error(Some(span), "Expected '|' for '||' operator");
                        continue;
                    }
                }
                None => {
                    tokens.push(Token {
                        kind: TokenKind::Eof,
                        span,
                    });
                    break;
                }
                Some(other) => {
                    self.bump();
                    self.diagnostics
                        .error(Some(span), format!("Unexpected character '{}'", other));
                    continue;
                }
            };

            tokens.push(token);
        }

        tokens
    }

    fn skip_ignored(&mut self) {
        loop {
            while matches!(self.current(), Some(ch) if ch.is_whitespace()) {
                self.bump();
            }

            if self.current() == Some('/') && self.peek() == Some('/') {
                while let Some(ch) = self.current() {
                    self.bump();
                    if ch == '\n' {
                        break;
                    }
                }
                continue;
            }

            break;
        }
    }

    fn lex_identifier(&mut self, span: Span) -> Token {
        let mut value = String::new();
        while let Some(ch) = self.current() {
            if is_ident_continue(ch) {
                value.push(ch);
                self.bump();
            } else {
                break;
            }
        }

        let kind = match value.as_str() {
            "import" => TokenKind::Import,
            "body" => TokenKind::Body,
            "let" => TokenKind::Let,
            "func" => TokenKind::Func,
            "return" => TokenKind::Return,
            "if" => TokenKind::If,
            "else" => TokenKind::Else,
            "while" => TokenKind::While,
            "do" => TokenKind::Do,
            "for" => TokenKind::For,
            "new" => TokenKind::New,
            "int" => TokenKind::IntType,
            "float" => TokenKind::FloatType,
            "string" => TokenKind::StringType,
            "bool" => TokenKind::BoolType,
            "true" => TokenKind::True,
            "false" => TokenKind::False,
            _ => TokenKind::Identifier(value),
        };

        Token { kind, span }
    }

    fn lex_number(&mut self, span: Span) -> Token {
        let mut value = String::new();
        let mut has_dot = false;

        while let Some(ch) = self.current() {
            if ch.is_ascii_digit() {
                value.push(ch);
                self.bump();
            } else if ch == '.' && !has_dot {
                has_dot = true;
                value.push(ch);
                self.bump();
            } else {
                break;
            }
        }

        let kind = if has_dot {
            match value.parse::<f64>() {
                Ok(number) => TokenKind::FloatLiteral(number),
                Err(_) => {
                    self.diagnostics
                        .error(Some(span), format!("Invalid float literal: {}", value));
                    TokenKind::FloatLiteral(0.0)
                }
            }
        } else {
            match value.parse::<i64>() {
                Ok(number) => TokenKind::IntLiteral(number),
                Err(_) => {
                    self.diagnostics
                        .error(Some(span), format!("Invalid int literal: {}", value));
                    TokenKind::IntLiteral(0)
                }
            }
        };

        Token { kind, span }
    }

    fn lex_string(&mut self, span: Span) -> Token {
        self.bump();
        let mut value = String::new();

        while let Some(ch) = self.current() {
            match ch {
                '"' => {
                    self.bump();
                    return Token {
                        kind: TokenKind::StringLiteral(value),
                        span,
                    };
                }
                '\\' => {
                    self.bump();
                    match self.current() {
                        Some('n') => {
                            value.push('\n');
                            self.bump();
                        }
                        Some('t') => {
                            value.push('\t');
                            self.bump();
                        }
                        Some('"') => {
                            value.push('"');
                            self.bump();
                        }
                        Some('(') => {
                            value.push('\\');
                            value.push('(');
                            self.bump();
                        }
                        Some('\\') => {
                            value.push('\\');
                            self.bump();
                        }
                        Some(other) => {
                            self.diagnostics
                                .error(Some(span), format!("Unknown escape sequence: \\{}", other));
                            value.push(other);
                            self.bump();
                        }
                        None => {
                            self.diagnostics
                                .error(Some(span), "Unterminated escape sequence");
                            break;
                        }
                    }
                }
                _ => {
                    value.push(ch);
                    self.bump();
                }
            }
        }

        self.diagnostics
            .error(Some(span), "Unterminated string literal");

        Token {
            kind: TokenKind::StringLiteral(value),
            span,
        }
    }

    fn current(&self) -> Option<char> {
        self.chars.get(self.index).copied()
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.index + 1).copied()
    }

    fn bump(&mut self) {
        if let Some(ch) = self.current() {
            self.index += 1;
            if ch == '\n' {
                self.line += 1;
                self.column = 1;
            } else {
                self.column += 1;
            }
        }
    }
}

fn is_ident_start(ch: char) -> bool {
    ch.is_ascii_alphabetic() || ch == '_'
}

fn is_ident_continue(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}
