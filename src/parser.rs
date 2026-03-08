use crate::ast::{
    AssignTarget, BinaryOp, BodyDecl, CallTarget, CapabilityDecl, Expr, FuncDecl, FuncParam,
    ImportDecl, Item, Program, RightDecl, Stmt, UnaryOp, ValueType, VarDecl,
};
use crate::diagnostics::Diagnostics;
use crate::lexer;
use crate::span::Span;
use crate::token::{Token, TokenKind};

pub fn parse(tokens: Vec<Token>, diagnostics: &mut Diagnostics) -> Option<Program> {
    let mut parser = Parser::new(tokens, diagnostics);
    parser.parse_program()
}

pub fn parse_expression_fragment(fragment: &str) -> Result<Expr, String> {
    let mut diagnostics = Diagnostics::default();
    let tokens = lexer::lex(fragment, &mut diagnostics);
    if diagnostics.has_errors() {
        let message = diagnostics
            .items()
            .first()
            .map(|item| item.message.clone())
            .unwrap_or_else(|| "Invalid interpolation expression".to_string());
        return Err(message);
    }

    let mut parser = Parser::new(tokens, &mut diagnostics);
    let Some(expr) = parser.parse_expression() else {
        let message = diagnostics
            .items()
            .first()
            .map(|item| item.message.clone())
            .unwrap_or_else(|| "Expected expression in interpolation".to_string());
        return Err(message);
    };

    if !matches!(parser.current().kind, TokenKind::Eof) {
        return Err("Unexpected tokens after interpolation expression".to_string());
    }

    if diagnostics.has_errors() {
        let message = diagnostics
            .items()
            .first()
            .map(|item| item.message.clone())
            .unwrap_or_else(|| "Invalid interpolation expression".to_string());
        return Err(message);
    }

    Ok(expr)
}

struct Parser<'a> {
    tokens: Vec<Token>,
    index: usize,
    diagnostics: &'a mut Diagnostics,
}

impl<'a> Parser<'a> {
    fn new(tokens: Vec<Token>, diagnostics: &'a mut Diagnostics) -> Self {
        Self {
            tokens,
            index: 0,
            diagnostics,
        }
    }

    fn parse_program(&mut self) -> Option<Program> {
        let mut imports = Vec::new();

        while self.check(|kind| matches!(kind, TokenKind::Import)) {
            let import_token = self.advance();
            let module = self.expect_identifier("Expected a module name after import")?;
            imports.push(ImportDecl {
                module,
                span: import_token.span,
            });
            self.consume_if(|kind| matches!(kind, TokenKind::Semicolon));
        }

        let body = self.parse_body_decl()?;

        Some(Program { imports, body })
    }

    fn parse_body_decl(&mut self) -> Option<BodyDecl> {
        let body_token = self.expect(
            |kind| matches!(kind, TokenKind::Body),
            "Expected 'body' keyword",
        )?;
        let body_name = self.expect_identifier("Expected body name")?;
        self.expect(
            |kind| matches!(kind, TokenKind::LBrace),
            "Expected '{' after body declaration",
        )?;

        let mut items = Vec::new();
        while !self.check(|kind| matches!(kind, TokenKind::RBrace | TokenKind::Eof)) {
            if let Some(item) = self.parse_item() {
                items.push(item);
            } else {
                self.synchronize_item();
            }
        }

        self.expect(
            |kind| matches!(kind, TokenKind::RBrace),
            "Expected '}' at the end of body",
        )?;

        Some(BodyDecl {
            name: body_name,
            items,
            span: body_token.span,
        })
    }

    fn parse_item(&mut self) -> Option<Item> {
        if self.check(|kind| matches!(kind, TokenKind::Let)) {
            self.parse_let_item()
        } else if self.check(|kind| matches!(kind, TokenKind::Func)) {
            self.parse_function_decl(None).map(Item::Func)
        } else {
            let token = self.advance();
            self.diagnostics.error(
                Some(token.span),
                "Expected let/func declaration inside body",
            );
            None
        }
    }

    fn parse_let_item(&mut self) -> Option<Item> {
        let let_token = self.expect(
            |kind| matches!(kind, TokenKind::Let),
            "Expected 'let' keyword",
        )?;

        let entitlement = if self
            .consume_if(|kind| matches!(kind, TokenKind::Colon))
            .is_some()
        {
            Some(self.expect_identifier("Expected entitlement name after ':'")?)
        } else {
            None
        };

        if self.check(|kind| matches!(kind, TokenKind::Func)) {
            return self.parse_function_decl(entitlement).map(Item::Func);
        }

        if entitlement.is_none() && self.check_identifier_text("capability") {
            self.advance();
            let name = self.expect_identifier("Expected capability name")?;
            self.expect(
                |kind| matches!(kind, TokenKind::Assign),
                "Expected '=' after capability name",
            )?;
            let initializer = self.parse_expression()?;
            self.consume_if(|kind| matches!(kind, TokenKind::Semicolon));
            return Some(Item::Capability(CapabilityDecl {
                name,
                initializer,
                span: let_token.span,
            }));
        }

        if entitlement.is_none() && self.check_identifier_text("right") {
            self.advance();
            let name = self.expect_identifier("Expected right name")?;
            self.expect(
                |kind| matches!(kind, TokenKind::Assign),
                "Expected '=' after right name",
            )?;
            let initializer = self.parse_expression()?;
            self.consume_if(|kind| matches!(kind, TokenKind::Semicolon));
            return Some(Item::Right(RightDecl {
                name,
                initializer,
                span: let_token.span,
            }));
        }

        self.parse_variable_decl_from_current(let_token.span, entitlement)
            .map(Item::Var)
    }

    fn parse_function_decl(&mut self, right: Option<String>) -> Option<FuncDecl> {
        let func_token = self.expect(
            |kind| matches!(kind, TokenKind::Func),
            "Expected 'func' keyword",
        )?;
        let name = self.expect_identifier("Expected function name")?;
        self.expect(
            |kind| matches!(kind, TokenKind::LParen),
            "Expected '(' after function name",
        )?;
        let params = self.parse_function_parameters()?;
        let return_type = if self
            .consume_if(|kind| matches!(kind, TokenKind::Arrow))
            .is_some()
        {
            let Some(parsed) = self.parse_type_annotation() else {
                self.diagnostics.error(
                    Some(self.current().span),
                    "Expected return type after '->'",
                );
                return None;
            };
            parsed
        } else {
            ValueType::Void
        };
        let body = self.parse_block_statements()?;

        Some(FuncDecl {
            name,
            params,
            return_type,
            right,
            body,
            span: func_token.span,
        })
    }

    fn parse_function_parameters(&mut self) -> Option<Vec<FuncParam>> {
        let mut params = Vec::new();
        if self
            .consume_if(|kind| matches!(kind, TokenKind::RParen))
            .is_some()
        {
            return Some(params);
        }

        loop {
            let param_span = self.current().span;
            let param_name = self.expect_identifier("Expected parameter name")?;
            self.expect(
                |kind| matches!(kind, TokenKind::Colon),
                "Expected ':' after parameter name",
            )?;
            let Some(param_type) = self.parse_type_annotation() else {
                self.diagnostics.error(
                    Some(self.current().span),
                    "Expected parameter type in function signature",
                );
                return None;
            };
            params.push(FuncParam {
                name: param_name,
                ty: param_type,
                span: param_span,
            });

            if self
                .consume_if(|kind| matches!(kind, TokenKind::Comma))
                .is_some()
            {
                continue;
            }

            self.expect(
                |kind| matches!(kind, TokenKind::RParen),
                "Expected ')' after function parameters",
            )?;
            break;
        }

        Some(params)
    }

    fn parse_block_statements(&mut self) -> Option<Vec<Stmt>> {
        self.expect(|kind| matches!(kind, TokenKind::LBrace), "Expected '{'")?;

        let mut statements = Vec::new();
        while !self.check(|kind| matches!(kind, TokenKind::RBrace | TokenKind::Eof)) {
            if let Some(stmt) = self.parse_statement() {
                statements.push(stmt);
            } else {
                self.synchronize_statement();
            }
        }

        self.expect(|kind| matches!(kind, TokenKind::RBrace), "Expected '}'")?;

        Some(statements)
    }

    fn parse_statement(&mut self) -> Option<Stmt> {
        if self.check(|kind| matches!(kind, TokenKind::Let)) {
            let let_token = self.advance();
            let entitlement = if self
                .consume_if(|kind| matches!(kind, TokenKind::Colon))
            .is_some()
            {
                Some(self.expect_identifier("Expected capability name after ':'")?)
            } else {
                None
            };

            if self.check(|kind| matches!(kind, TokenKind::Func))
                || self.check_identifier_text("capability")
                || self.check_identifier_text("right")
            {
                self.diagnostics.error(
                    Some(let_token.span),
                    "Only variable declarations with let are allowed inside function body",
                );
                return None;
            }

            return self
                .parse_variable_decl_from_current(let_token.span, entitlement)
                .map(Stmt::VarDecl);
        }

        if self.check(|kind| matches!(kind, TokenKind::If)) {
            return self.parse_if_statement();
        }

        if self.check(|kind| matches!(kind, TokenKind::While)) {
            return self.parse_while_statement();
        }

        if self.check(|kind| matches!(kind, TokenKind::Do)) {
            return self.parse_do_while_statement();
        }

        if self.check(|kind| matches!(kind, TokenKind::For)) {
            return self.parse_for_statement();
        }

        if self.check(|kind| matches!(kind, TokenKind::LBrace)) {
            let block_span = self.current().span;
            let statements = self.parse_block_statements()?;
            return Some(Stmt::Block {
                statements,
                span: block_span,
            });
        }

        if self.check(|kind| matches!(kind, TokenKind::Return)) {
            let return_token = self.advance();
            if self
                .consume_if(|kind| matches!(kind, TokenKind::Semicolon))
                .is_some()
            {
                return Some(Stmt::Return {
                    value: None,
                    span: return_token.span,
                });
            }
            let value = self.parse_expression()?;
            self.consume_if(|kind| matches!(kind, TokenKind::Semicolon));
            return Some(Stmt::Return {
                value: Some(value),
                span: return_token.span,
            });
        }

        let expr = self.parse_expression()?;
        if self
            .consume_if(|kind| matches!(kind, TokenKind::Assign))
            .is_some()
        {
            let span = expr.span();
            let Some(target) = self.expression_to_assign_target(expr) else {
                self.diagnostics.error(
                    Some(span),
                    "Invalid assignment target. Expected variable or array index",
                );
                return None;
            };
            let value = self.parse_expression()?;
            self.consume_if(|kind| matches!(kind, TokenKind::Semicolon));
            return Some(Stmt::Assign {
                target,
                value,
                span,
            });
        }

        self.consume_if(|kind| matches!(kind, TokenKind::Semicolon));
        let expr_span = expr.span();
        Some(Stmt::Expr {
            expr,
            span: expr_span,
        })
    }

    fn parse_if_statement(&mut self) -> Option<Stmt> {
        let if_token = self.expect(
            |kind| matches!(kind, TokenKind::If),
            "Expected 'if' keyword",
        )?;
        self.expect(
            |kind| matches!(kind, TokenKind::LParen),
            "Expected '(' after if",
        )?;
        let condition = self.parse_expression()?;
        self.expect(
            |kind| matches!(kind, TokenKind::RParen),
            "Expected ')' after if condition",
        )?;

        let then_branch = self.parse_block_statements()?;
        let else_branch = if self
            .consume_if(|kind| matches!(kind, TokenKind::Else))
            .is_some()
        {
            if self.check(|kind| matches!(kind, TokenKind::If)) {
                vec![self.parse_if_statement()?]
            } else {
                self.parse_block_statements()?
            }
        } else {
            Vec::new()
        };

        Some(Stmt::If {
            condition,
            then_branch,
            else_branch,
            span: if_token.span,
        })
    }

    fn parse_while_statement(&mut self) -> Option<Stmt> {
        let while_token = self.expect(
            |kind| matches!(kind, TokenKind::While),
            "Expected 'while' keyword",
        )?;
        self.expect(
            |kind| matches!(kind, TokenKind::LParen),
            "Expected '(' after while",
        )?;
        let condition = self.parse_expression()?;
        self.expect(
            |kind| matches!(kind, TokenKind::RParen),
            "Expected ')' after while condition",
        )?;
        let body = self.parse_block_statements()?;
        Some(Stmt::While {
            condition,
            body,
            span: while_token.span,
        })
    }

    fn parse_do_while_statement(&mut self) -> Option<Stmt> {
        let do_token = self.expect(
            |kind| matches!(kind, TokenKind::Do),
            "Expected 'do' keyword",
        )?;
        let body = self.parse_block_statements()?;
        self.expect(
            |kind| matches!(kind, TokenKind::While),
            "Expected 'while' after do-block",
        )?;
        self.expect(
            |kind| matches!(kind, TokenKind::LParen),
            "Expected '(' after while in do-while",
        )?;
        let condition = self.parse_expression()?;
        self.expect(
            |kind| matches!(kind, TokenKind::RParen),
            "Expected ')' after do-while condition",
        )?;
        self.consume_if(|kind| matches!(kind, TokenKind::Semicolon));
        Some(Stmt::DoWhile {
            body,
            condition,
            span: do_token.span,
        })
    }

    fn parse_for_statement(&mut self) -> Option<Stmt> {
        let for_token = self.expect(
            |kind| matches!(kind, TokenKind::For),
            "Expected 'for' keyword",
        )?;
        self.expect(
            |kind| matches!(kind, TokenKind::LParen),
            "Expected '(' after for",
        )?;

        let init = self.parse_for_header_clause(true)?;
        let condition = if self
            .consume_if(|kind| matches!(kind, TokenKind::Semicolon))
            .is_some()
        {
            None
        } else {
            let expr = self.parse_expression()?;
            self.expect(
                |kind| matches!(kind, TokenKind::Semicolon),
                "Expected ';' after for condition",
            )?;
            Some(expr)
        };

        let update = self.parse_for_header_clause(false)?;
        self.expect(
            |kind| matches!(kind, TokenKind::RParen),
            "Expected ')' after for clauses",
        )?;
        let body = self.parse_block_statements()?;

        let mut while_body = body;
        if let Some(update_stmt) = update {
            while_body.push(update_stmt);
        }

        let while_stmt = Stmt::While {
            condition: condition.unwrap_or(Expr::BoolLiteral(true, for_token.span)),
            body: while_body,
            span: for_token.span,
        };

        if let Some(init_stmt) = init {
            Some(Stmt::Block {
                statements: vec![init_stmt, while_stmt],
                span: for_token.span,
            })
        } else {
            Some(while_stmt)
        }
    }

    fn parse_for_header_clause(&mut self, expect_semicolon: bool) -> Option<Option<Stmt>> {
        if expect_semicolon {
            if self.check(|kind| matches!(kind, TokenKind::Semicolon)) {
                self.advance();
                return Some(None);
            }
        } else if self.check(|kind| matches!(kind, TokenKind::RParen)) {
            return Some(None);
        }

        if self.check(|kind| matches!(kind, TokenKind::Let)) {
            let let_token = self.advance();
            let entitlement = if self
                .consume_if(|kind| matches!(kind, TokenKind::Colon))
                .is_some()
            {
                Some(self.expect_identifier("Expected entitlement name after ':'")?)
            } else {
                None
            };

            if self.check(|kind| matches!(kind, TokenKind::Func))
                || self.check_identifier_text("capability")
                || self.check_identifier_text("right")
            {
                self.diagnostics.error(
                    Some(let_token.span),
                    "Only variable declarations with let are allowed in for-clause",
                );
                return None;
            }

            if !expect_semicolon {
                self.diagnostics.error(
                    Some(let_token.span),
                    "for update clause cannot contain let-declaration",
                );
                return None;
            }

            let explicit_type = self.parse_type_annotation();
            let name = self.expect_identifier("Expected variable name")?;
            self.expect(
                |kind| matches!(kind, TokenKind::Assign),
                "Expected '=' in variable declaration",
            )?;
            let initializer = self.parse_expression()?;
            self.expect(
                |kind| matches!(kind, TokenKind::Semicolon),
                "Expected ';' after for init declaration",
            )?;

            let decl = VarDecl {
                name,
                explicit_type,
                initializer,
                entitlement,
                span: let_token.span,
            };
            return Some(Some(Stmt::VarDecl(decl)));
        }

        let expr = self.parse_expression()?;
        let statement = if self
            .consume_if(|kind| matches!(kind, TokenKind::Assign))
            .is_some()
        {
            let span = expr.span();
            let Some(target) = self.expression_to_assign_target(expr) else {
                self.diagnostics.error(
                    Some(span),
                    "Invalid assignment target in for-clause",
                );
                return None;
            };
            let value = self.parse_expression()?;
            Stmt::Assign {
                target,
                value,
                span,
            }
        } else {
            let span = expr.span();
            Stmt::Expr { expr, span }
        };

        if expect_semicolon {
            self.expect(
                |kind| matches!(kind, TokenKind::Semicolon),
                "Expected ';' after for init clause",
            )?;
        }

        Some(Some(statement))
    }

    fn parse_variable_decl_from_current(
        &mut self,
        span: Span,
        entitlement: Option<String>,
    ) -> Option<VarDecl> {
        let explicit_type = self.parse_type_annotation();

        let name = self.expect_identifier("Expected variable name")?;
        self.expect(
            |kind| matches!(kind, TokenKind::Assign),
            "Expected '=' in variable declaration",
        )?;
        let initializer = self.parse_expression()?;
        self.consume_if(|kind| matches!(kind, TokenKind::Semicolon));

        Some(VarDecl {
            name,
            explicit_type,
            initializer,
            entitlement,
            span,
        })
    }

    fn parse_type_annotation(&mut self) -> Option<ValueType> {
        let kind = &self.current().kind;
        let mut result = match kind {
            TokenKind::IntType => Some(ValueType::Int),
            TokenKind::FloatType => Some(ValueType::Float),
            TokenKind::StringType => Some(ValueType::String),
            TokenKind::BoolType => Some(ValueType::Bool),
            _ => None,
        };

        if result.is_some() {
            self.advance();
        }

        while result.is_some()
            && self
                .consume_if(|kind| matches!(kind, TokenKind::LBracket))
                .is_some()
        {
            self.expect(
                |kind| matches!(kind, TokenKind::RBracket),
                "Expected ']' in array type declaration",
            )?;
            let inner = result.take().expect("type is present");
            result = Some(ValueType::Array(Box::new(inner)));
        }

        result
    }

    fn parse_expression(&mut self) -> Option<Expr> {
        self.parse_ternary()
    }

    fn parse_ternary(&mut self) -> Option<Expr> {
        let condition = self.parse_or()?;
        if self
            .consume_if(|kind| matches!(kind, TokenKind::Question))
            .is_none()
        {
            return Some(condition);
        }

        let then_expr = self.parse_expression()?;
        self.expect(
            |kind| matches!(kind, TokenKind::Colon),
            "Expected ':' in ternary operator",
        )?;
        let else_expr = self.parse_expression()?;
        let span = condition.span();

        Some(Expr::Ternary {
            condition: Box::new(condition),
            then_expr: Box::new(then_expr),
            else_expr: Box::new(else_expr),
            span,
        })
    }

    fn parse_or(&mut self) -> Option<Expr> {
        let mut expr = self.parse_and()?;
        while self
            .consume_if(|kind| matches!(kind, TokenKind::OrOr))
            .is_some()
        {
            let right = self.parse_and()?;
            let span = expr.span();
            expr = Expr::Binary {
                left: Box::new(expr),
                op: BinaryOp::Or,
                right: Box::new(right),
                span,
            };
        }
        Some(expr)
    }

    fn parse_and(&mut self) -> Option<Expr> {
        let mut expr = self.parse_equality()?;
        while self
            .consume_if(|kind| matches!(kind, TokenKind::AndAnd))
            .is_some()
        {
            let right = self.parse_equality()?;
            let span = expr.span();
            expr = Expr::Binary {
                left: Box::new(expr),
                op: BinaryOp::And,
                right: Box::new(right),
                span,
            };
        }
        Some(expr)
    }

    fn parse_equality(&mut self) -> Option<Expr> {
        let mut expr = self.parse_comparison()?;

        loop {
            let op = if self
                .consume_if(|kind| matches!(kind, TokenKind::EqEq))
                .is_some()
            {
                Some(BinaryOp::Eq)
            } else if self
                .consume_if(|kind| matches!(kind, TokenKind::NotEq))
                .is_some()
            {
                Some(BinaryOp::NotEq)
            } else {
                None
            };

            let Some(op) = op else { break };
            let right = self.parse_comparison()?;
            let span = expr.span();
            expr = Expr::Binary {
                left: Box::new(expr),
                op,
                right: Box::new(right),
                span,
            };
        }

        Some(expr)
    }

    fn parse_comparison(&mut self) -> Option<Expr> {
        let mut expr = self.parse_term()?;

        loop {
            let op = if self
                .consume_if(|kind| matches!(kind, TokenKind::Greater))
                .is_some()
            {
                Some(BinaryOp::Greater)
            } else if self
                .consume_if(|kind| matches!(kind, TokenKind::GreaterEq))
                .is_some()
            {
                Some(BinaryOp::GreaterEq)
            } else if self
                .consume_if(|kind| matches!(kind, TokenKind::Less))
                .is_some()
            {
                Some(BinaryOp::Less)
            } else if self
                .consume_if(|kind| matches!(kind, TokenKind::LessEq))
                .is_some()
            {
                Some(BinaryOp::LessEq)
            } else {
                None
            };

            let Some(op) = op else { break };
            let right = self.parse_term()?;
            let span = expr.span();
            expr = Expr::Binary {
                left: Box::new(expr),
                op,
                right: Box::new(right),
                span,
            };
        }

        Some(expr)
    }

    fn parse_term(&mut self) -> Option<Expr> {
        let mut expr = self.parse_factor()?;

        loop {
            let op = if self
                .consume_if(|kind| matches!(kind, TokenKind::Plus))
                .is_some()
            {
                Some(BinaryOp::Add)
            } else if self
                .consume_if(|kind| matches!(kind, TokenKind::Minus))
                .is_some()
            {
                Some(BinaryOp::Sub)
            } else {
                None
            };

            let Some(op) = op else { break };
            let right = self.parse_factor()?;
            let span = expr.span();
            expr = Expr::Binary {
                left: Box::new(expr),
                op,
                right: Box::new(right),
                span,
            };
        }

        Some(expr)
    }

    fn parse_factor(&mut self) -> Option<Expr> {
        let mut expr = self.parse_unary()?;

        loop {
            let op = if self
                .consume_if(|kind| matches!(kind, TokenKind::Star))
                .is_some()
            {
                Some(BinaryOp::Mul)
            } else if self
                .consume_if(|kind| matches!(kind, TokenKind::Slash))
                .is_some()
            {
                Some(BinaryOp::Div)
            } else if self
                .consume_if(|kind| matches!(kind, TokenKind::Percent))
                .is_some()
            {
                Some(BinaryOp::Mod)
            } else {
                None
            };

            let Some(op) = op else { break };
            let right = self.parse_unary()?;
            let span = expr.span();
            expr = Expr::Binary {
                left: Box::new(expr),
                op,
                right: Box::new(right),
                span,
            };
        }

        Some(expr)
    }

    fn parse_unary(&mut self) -> Option<Expr> {
        if self
            .consume_if(|kind| matches!(kind, TokenKind::Bang))
            .is_some()
        {
            let expr = self.parse_unary()?;
            let span = expr.span();
            return Some(Expr::Unary {
                op: UnaryOp::Not,
                expr: Box::new(expr),
                span,
            });
        }

        if self
            .consume_if(|kind| matches!(kind, TokenKind::Minus))
            .is_some()
        {
            let expr = self.parse_unary()?;
            let span = expr.span();
            return Some(Expr::Unary {
                op: UnaryOp::Neg,
                expr: Box::new(expr),
                span,
            });
        }

        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Option<Expr> {
        let token = self.advance();

        let expr = match token.kind {
            TokenKind::IntLiteral(value) => Some(Expr::IntLiteral(value, token.span)),
            TokenKind::FloatLiteral(value) => Some(Expr::FloatLiteral(value, token.span)),
            TokenKind::StringLiteral(value) => Some(Expr::StringLiteral(value, token.span)),
            TokenKind::True => Some(Expr::BoolLiteral(true, token.span)),
            TokenKind::False => Some(Expr::BoolLiteral(false, token.span)),
            TokenKind::Identifier(name) => {
                if self
                    .consume_if(|kind| matches!(kind, TokenKind::Dot))
                    .is_some()
                {
                    let method_name =
                        self.expect_identifier("Expected name after '.' in qualified call")?;
                    self.expect(
                        |kind| matches!(kind, TokenKind::LParen),
                        "Expected '(' after method name",
                    )?;
                    let args = self.parse_arguments()?;
                    Some(Expr::Call {
                        callee: CallTarget::Qualified {
                            module: name,
                            name: method_name,
                        },
                        args,
                        span: token.span,
                    })
                } else if self
                    .consume_if(|kind| matches!(kind, TokenKind::LParen))
                    .is_some()
                {
                    let args = self.parse_arguments()?;
                    Some(Expr::Call {
                        callee: CallTarget::Name(name),
                        args,
                        span: token.span,
                    })
                } else {
                    Some(Expr::Identifier(name, token.span))
                }
            }
            TokenKind::New => {
                let kind = self.expect_identifier("Expected constructor name after new")?;
                self.expect(
                    |token_kind| matches!(token_kind, TokenKind::LParen),
                    "Expected '(' after constructor name",
                )?;
                let args = self.parse_arguments()?;
                Some(Expr::NewObject {
                    kind,
                    args,
                    span: token.span,
                })
            }
            TokenKind::LBracket => {
                let mut elements = Vec::new();
                if self
                    .consume_if(|kind| matches!(kind, TokenKind::RBracket))
                    .is_none()
                {
                    loop {
                        elements.push(self.parse_expression()?);
                        if self
                            .consume_if(|kind| matches!(kind, TokenKind::Comma))
                            .is_some()
                        {
                            continue;
                        }
                        self.expect(
                            |kind| matches!(kind, TokenKind::RBracket),
                            "Expected ']' after array literal",
                        )?;
                        break;
                    }
                }
                Some(Expr::ArrayLiteral(elements, token.span))
            }
            TokenKind::LParen => {
                let expr = self.parse_expression()?;
                self.expect(|kind| matches!(kind, TokenKind::RParen), "Expected ')'")?;
                Some(expr)
            }
            _ => {
                self.diagnostics
                    .error(Some(token.span), "Expected expression");
                None
            }
        }?;

        self.parse_postfix(expr)
    }

    fn parse_arguments(&mut self) -> Option<Vec<Expr>> {
        let mut args = Vec::new();

        if self
            .consume_if(|kind| matches!(kind, TokenKind::RParen))
            .is_some()
        {
            return Some(args);
        }

        loop {
            args.push(self.parse_expression()?);
            if self
                .consume_if(|kind| matches!(kind, TokenKind::Comma))
                .is_some()
            {
                continue;
            }
            self.expect(
                |kind| matches!(kind, TokenKind::RParen),
                "Expected ')' after arguments",
            )?;
            break;
        }

        Some(args)
    }

    fn parse_postfix(&mut self, mut expr: Expr) -> Option<Expr> {
        while self
            .consume_if(|kind| matches!(kind, TokenKind::LBracket))
            .is_some()
        {
            let index = self.parse_expression()?;
            self.expect(
                |kind| matches!(kind, TokenKind::RBracket),
                "Expected ']' after index expression",
            )?;
            let span = expr.span();
            expr = Expr::Index {
                array: Box::new(expr),
                index: Box::new(index),
                span,
            };
        }

        Some(expr)
    }

    fn expression_to_assign_target(&mut self, expr: Expr) -> Option<AssignTarget> {
        match expr {
            Expr::Identifier(name, _) => Some(AssignTarget::Identifier(name)),
            Expr::Index { array, index, .. } => Some(AssignTarget::Index {
                array: *array,
                index: *index,
            }),
            _ => None,
        }
    }

    fn check_identifier_text(&self, expected: &str) -> bool {
        matches!(&self.current().kind, TokenKind::Identifier(text) if text == expected)
    }

    fn expect_identifier(&mut self, message: &str) -> Option<String> {
        let token = self.advance();
        match token.kind {
            TokenKind::Identifier(value) => Some(value),
            _ => {
                self.diagnostics.error(Some(token.span), message);
                None
            }
        }
    }

    fn expect<F>(&mut self, predicate: F, message: &str) -> Option<Token>
    where
        F: Fn(&TokenKind) -> bool,
    {
        if self.check(predicate) {
            Some(self.advance())
        } else {
            self.diagnostics.error(Some(self.current().span), message);
            None
        }
    }

    fn consume_if<F>(&mut self, predicate: F) -> Option<Token>
    where
        F: Fn(&TokenKind) -> bool,
    {
        if self.check(predicate) {
            Some(self.advance())
        } else {
            None
        }
    }

    fn check<F>(&self, predicate: F) -> bool
    where
        F: Fn(&TokenKind) -> bool,
    {
        predicate(&self.current().kind)
    }

    fn advance(&mut self) -> Token {
        let token = self
            .tokens
            .get(self.index)
            .cloned()
            .unwrap_or_else(|| Token {
                kind: TokenKind::Eof,
                span: Span::new(0, 0),
            });

        if !matches!(token.kind, TokenKind::Eof) {
            self.index += 1;
        }

        token
    }

    fn current(&self) -> &Token {
        self.tokens.get(self.index).unwrap_or_else(|| {
            self.tokens
                .last()
                .expect("Parser requires at least one EOF token")
        })
    }

    fn synchronize_item(&mut self) {
        while !matches!(
            self.current().kind,
            TokenKind::Eof | TokenKind::RBrace | TokenKind::Let | TokenKind::Func
        ) {
            self.advance();
        }
    }

    fn synchronize_statement(&mut self) {
        while !matches!(
            self.current().kind,
            TokenKind::Eof
                | TokenKind::RBrace
                | TokenKind::Semicolon
                | TokenKind::If
                | TokenKind::While
                | TokenKind::Do
                | TokenKind::For
                | TokenKind::Let
                | TokenKind::Return
        ) {
            self.advance();
        }

        self.consume_if(|kind| matches!(kind, TokenKind::Semicolon));
    }
}
