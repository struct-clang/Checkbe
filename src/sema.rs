use std::collections::HashMap;

use crate::ast::{
    AssignTarget, BinaryOp, CallTarget, CapabilityDecl, Expr, FuncDecl, Item, Program, RightDecl,
    Stmt, UnaryOp, ValueType, VarDecl,
};
use crate::diagnostics::Diagnostics;
use crate::module_system::ModuleRegistry;
use crate::parser::parse_expression_fragment;
use crate::span::Span;
use crate::string_interp::{parse_segments, Segment};

#[derive(Clone, Copy, Debug)]
enum Operation {
    Read,
    Write,
}

#[derive(Clone, Debug)]
pub struct PermissionSet {
    pub raw: Vec<String>,
    allow_read: bool,
    allow_write: bool,
    allow_all: bool,
    deny_read: bool,
    deny_write: bool,
    deny_all: bool,
}

impl PermissionSet {
    fn from_atoms(atoms: Vec<String>) -> (Self, Vec<String>) {
        let mut warnings = Vec::new();
        let mut set = Self {
            raw: atoms.clone(),
            allow_read: false,
            allow_write: false,
            allow_all: false,
            deny_read: false,
            deny_write: false,
            deny_all: false,
        };

        for atom in atoms {
            let lowered = atom.to_ascii_lowercase();
            let deny = lowered.contains("forbid")
                || lowered.contains("deny")
                || lowered.contains("no_")
                || lowered.starts_with('!');
            let mut recognized = false;

            if lowered.contains("all") || lowered.contains("full") {
                recognized = true;
                if deny {
                    set.deny_all = true;
                } else {
                    set.allow_all = true;
                }
            }

            if lowered.contains("read") {
                recognized = true;
                if deny {
                    set.deny_read = true;
                } else {
                    set.allow_read = true;
                }
            }

            if lowered.contains("write") {
                recognized = true;
                if deny {
                    set.deny_write = true;
                } else {
                    set.allow_write = true;
                }
            }

            if !recognized {
                warnings.push(format!(
                    "Permission '{}' does not affect read/write/all and will be ignored",
                    atom
                ));
            }
        }

        if set.allow_read && set.deny_read {
            warnings.push("Conflicting permissions: both allow read and deny read".to_string());
        }
        if set.allow_write && set.deny_write {
            warnings.push("Conflicting permissions: both allow write and deny write".to_string());
        }

        (set, warnings)
    }

    fn allows(&self, operation: Operation) -> bool {
        if self.deny_all {
            return false;
        }

        match operation {
            Operation::Read => {
                if self.deny_read {
                    return false;
                }
                self.allow_all || self.allow_read
            }
            Operation::Write => {
                if self.deny_write {
                    return false;
                }
                self.allow_all || self.allow_write
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct VariableInfo {
    pub ty: ValueType,
    pub entitlement: Option<String>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct FunctionInfo {
    pub params: Vec<ValueType>,
    pub return_type: ValueType,
}

#[derive(Clone, Debug)]
pub struct SemanticModel {
    pub globals: HashMap<String, VariableInfo>,
    pub functions: HashMap<String, FunctionInfo>,
    pub modules: ModuleRegistry,
}

pub fn analyze(
    program: &Program,
    modules: ModuleRegistry,
    diagnostics: &mut Diagnostics,
) -> Option<SemanticModel> {
    let mut analyzer = SemanticAnalyzer::new(program, modules, diagnostics);
    analyzer.analyze_program()
}

struct SemanticAnalyzer<'a> {
    program: &'a Program,
    diagnostics: &'a mut Diagnostics,
    capabilities: HashMap<String, PermissionSet>,
    rights: HashMap<String, PermissionSet>,
    globals: HashMap<String, VariableInfo>,
    functions: HashMap<String, FunctionInfo>,
    modules: ModuleRegistry,
}

impl<'a> SemanticAnalyzer<'a> {
    fn new(
        program: &'a Program,
        modules: ModuleRegistry,
        diagnostics: &'a mut Diagnostics,
    ) -> Self {
        Self {
            program,
            diagnostics,
            capabilities: HashMap::new(),
            rights: HashMap::new(),
            globals: HashMap::new(),
            functions: HashMap::new(),
            modules,
        }
    }

    fn analyze_program(&mut self) -> Option<SemanticModel> {
        for item in &self.program.body.items {
            match item {
                Item::Capability(decl) => self.collect_capability(decl),
                Item::Right(decl) => self.collect_right(decl),
                _ => {}
            }
        }

        for item in &self.program.body.items {
            match item {
                Item::Var(decl) => self.collect_global_var_signature(decl),
                Item::Func(decl) => self.collect_function_signature(decl),
                _ => {}
            }
        }

        if !self.functions.contains_key("main") {
            self.diagnostics
                .error(None, "Required function main() is missing")
        }

        for item in &self.program.body.items {
            if let Item::Var(decl) = item {
                self.analyze_global_var_initializer(decl);
            }
        }

        for item in &self.program.body.items {
            if let Item::Func(func) = item {
                self.analyze_function(func);
            }
        }

        if self.diagnostics.has_errors() {
            return None;
        }

        Some(SemanticModel {
            globals: self.globals.clone(),
            functions: self.functions.clone(),
            modules: self.modules.clone(),
        })
    }

    fn collect_capability(&mut self, decl: &CapabilityDecl) {
        if self.capabilities.contains_key(&decl.name) {
            self.diagnostics.error(
                Some(decl.span),
                format!("Capability '{}' is already declared", decl.name),
            );
            return;
        }

        let Some(atoms) = self.extract_permission_atoms(&decl.initializer, "Capability") else {
            return;
        };

        let (set, warnings) = PermissionSet::from_atoms(atoms);
        for warning in warnings {
            self.diagnostics.warning(Some(decl.span), warning);
        }

        self.capabilities.insert(decl.name.clone(), set);
    }

    fn collect_right(&mut self, decl: &RightDecl) {
        if self.rights.contains_key(&decl.name) {
            self.diagnostics.error(
                Some(decl.span),
                format!("Right '{}' is already declared", decl.name),
            );
            return;
        }

        let Some(atoms) = self.extract_permission_atoms(&decl.initializer, "Right") else {
            return;
        };

        let (set, warnings) = PermissionSet::from_atoms(atoms);
        for warning in warnings {
            self.diagnostics.warning(Some(decl.span), warning);
        }

        self.rights.insert(decl.name.clone(), set);
    }

    fn extract_permission_atoms(
        &mut self,
        initializer: &Expr,
        expected_kind: &str,
    ) -> Option<Vec<String>> {
        let Expr::NewObject { kind, args, span } = initializer else {
            self.diagnostics.error(
                Some(initializer.span()),
                format!(
                    "Expected constructor of form new {}(...), got a different expression",
                    expected_kind
                ),
            );
            return None;
        };

        if kind != expected_kind {
            self.diagnostics.warning(
                Some(*span),
                format!(
                    "Constructor '{}' is used for {}, expected '{}'",
                    kind, expected_kind, expected_kind
                ),
            );
        }

        let mut atoms = Vec::new();
        for arg in args {
            match arg {
                Expr::Identifier(name, _) | Expr::StringLiteral(name, _) => {
                    atoms.push(name.clone())
                }
                _ => {
                    self.diagnostics.error(
                        Some(arg.span()),
                        "Only identifiers or string literals are allowed in new Capability/new Right",
                    );
                    return None;
                }
            }
        }

        if atoms.is_empty() {
            self.diagnostics.warning(
                Some(*span),
                format!(
                    "{} declared without permissions (access denied by default)",
                    expected_kind
                ),
            );
        }

        Some(atoms)
    }

    fn collect_global_var_signature(&mut self, decl: &VarDecl) {
        if self.globals.contains_key(&decl.name) {
            self.diagnostics.error(
                Some(decl.span),
                format!("Variable '{}' is already declared", decl.name),
            );
            return;
        }

        if let Some(capability) = &decl.entitlement {
            if !self.capabilities.contains_key(capability) {
                self.diagnostics.error(
                    Some(decl.span),
                    format!("Capability '{}' not found", capability),
                );
            }
        }

        self.globals.insert(
            decl.name.clone(),
            VariableInfo {
                ty: ValueType::Void,
                entitlement: decl.entitlement.clone(),
                span: decl.span,
            },
        );
    }

    fn collect_function_signature(&mut self, decl: &FuncDecl) {
        if self.functions.contains_key(&decl.name) {
            self.diagnostics.error(
                Some(decl.span),
                format!("Function '{}' is already declared", decl.name),
            );
            return;
        }

        if decl.name == "main" {
            if decl.right.is_some() {
                self.diagnostics.warning(
                    Some(decl.span),
                    "Rights on main are ignored; main always has full rights",
                );
            }
            if !decl.params.is_empty() {
                self.diagnostics
                    .error(Some(decl.span), "main() must not have parameters");
            }
            if decl.return_type != ValueType::Void {
                self.diagnostics
                    .error(Some(decl.span), "main() must have return type void");
            }
        } else if let Some(right_name) = &decl.right {
            if !self.rights.contains_key(right_name) {
                self.diagnostics
                    .error(Some(decl.span), format!("Right '{}' not found", right_name));
            }
        }

        let mut params = Vec::with_capacity(decl.params.len());
        for param in &decl.params {
            params.push(param.ty.clone());
        }

        self.functions.insert(
            decl.name.clone(),
            FunctionInfo {
                params,
                return_type: decl.return_type.clone(),
            },
        );
    }

    fn analyze_global_var_initializer(&mut self, decl: &VarDecl) {
        let mut scope = HashMap::new();
        for (name, info) in &self.globals {
            if name != &decl.name && info.ty != ValueType::Void {
                scope.insert(name.clone(), info.clone());
            }
        }

        let Some(expr_type) = self.analyze_expr(&decl.initializer, &scope, None) else {
            return;
        };

        let resolved_type = if let Some(explicit_type) = &decl.explicit_type {
            if !is_assignable(explicit_type, &expr_type) {
                self.diagnostics.error(
                    Some(decl.span),
                    format!(
                        "Initializer type '{}' is not compatible with declared type '{}'",
                        expr_type.as_str(),
                        explicit_type.as_str()
                    ),
                );
                explicit_type.clone()
            } else {
                explicit_type.clone()
            }
        } else {
            expr_type
        };

        if let Some(info) = self.globals.get_mut(&decl.name) {
            info.ty = resolved_type;
        }
    }

    fn analyze_function(&mut self, decl: &FuncDecl) {
        let right: Option<PermissionSet> = if decl.name == "main" {
            None
        } else {
            decl.right
                .as_ref()
                .and_then(|right_name| self.rights.get(right_name))
                .cloned()
        };

        let mut locals = HashMap::new();
        for param in &decl.params {
            if locals.contains_key(&param.name) {
                self.diagnostics.error(
                    Some(param.span),
                    format!("Duplicate parameter '{}'", param.name),
                );
                continue;
            }

            locals.insert(
                param.name.clone(),
                VariableInfo {
                    ty: param.ty.clone(),
                    entitlement: None,
                    span: param.span,
                },
            );
        }

        let mut has_return = false;
        for statement in &decl.body {
            self.analyze_statement(
                statement,
                &mut locals,
                right.as_ref(),
                &decl.return_type,
                &mut has_return,
            );
        }

        if decl.return_type != ValueType::Void && !has_return {
            self.diagnostics.error(
                Some(decl.span),
                format!(
                    "Function '{}' must return '{}' on at least one path",
                    decl.name,
                    decl.return_type.as_str()
                ),
            );
        }
    }

    fn analyze_statement(
        &mut self,
        statement: &Stmt,
        locals: &mut HashMap<String, VariableInfo>,
        right: Option<&PermissionSet>,
        function_return_type: &ValueType,
        has_return: &mut bool,
    ) {
        match statement {
            Stmt::VarDecl(decl) => self.analyze_local_var_decl(decl, locals, right),
            Stmt::Assign {
                target,
                value,
                span,
            } => {
                let target_info = self.assignment_target_info(target, locals, right, *span);
                if let Some((owning_variable, target_type)) = target_info {
                    if let Some(value_type) = self.analyze_expr(value, locals, right) {
                        if !is_assignable(&target_type, &value_type) {
                            self.diagnostics.error(
                                Some(*span),
                                format!(
                                    "Cannot assign '{}' to target of type '{}'",
                                    value_type.as_str(),
                                    target_type.as_str()
                                ),
                            );
                        }
                    }
                    self.enforce_write_access(&owning_variable, right, *span);
                }
            }
            Stmt::If {
                condition,
                then_branch,
                else_branch,
                span,
            } => {
                if let Some(condition_type) = self.analyze_expr(condition, locals, right) {
                    if condition_type != ValueType::Bool {
                        self.diagnostics.error(
                            Some(*span),
                            format!(
                                "If condition must be bool, got '{}'",
                                condition_type.as_str()
                            ),
                        );
                    }
                }

                let mut then_locals = locals.clone();
                for nested in then_branch {
                    self.analyze_statement(
                        nested,
                        &mut then_locals,
                        right,
                        function_return_type,
                        has_return,
                    );
                }

                let mut else_locals = locals.clone();
                for nested in else_branch {
                    self.analyze_statement(
                        nested,
                        &mut else_locals,
                        right,
                        function_return_type,
                        has_return,
                    );
                }
            }
            Stmt::While {
                condition,
                body,
                span,
            } => {
                if let Some(condition_type) = self.analyze_expr(condition, locals, right) {
                    if condition_type != ValueType::Bool {
                        self.diagnostics.error(
                            Some(*span),
                            format!(
                                "While condition must be bool, got '{}'",
                                condition_type.as_str()
                            ),
                        );
                    }
                }

                let mut loop_locals = locals.clone();
                for nested in body {
                    self.analyze_statement(
                        nested,
                        &mut loop_locals,
                        right,
                        function_return_type,
                        has_return,
                    );
                }
            }
            Stmt::DoWhile {
                body,
                condition,
                span,
            } => {
                let mut loop_locals = locals.clone();
                for nested in body {
                    self.analyze_statement(
                        nested,
                        &mut loop_locals,
                        right,
                        function_return_type,
                        has_return,
                    );
                }

                if let Some(condition_type) = self.analyze_expr(condition, locals, right) {
                    if condition_type != ValueType::Bool {
                        self.diagnostics.error(
                            Some(*span),
                            format!(
                                "Do-while condition must be bool, got '{}'",
                                condition_type.as_str()
                            ),
                        );
                    }
                }
            }
            Stmt::Expr { expr, .. } => {
                self.analyze_expr(expr, locals, right);
            }
            Stmt::Block { statements, .. } => {
                let mut nested = locals.clone();
                for nested_stmt in statements {
                    self.analyze_statement(
                        nested_stmt,
                        &mut nested,
                        right,
                        function_return_type,
                        has_return,
                    );
                }
            }
            Stmt::Return { value, span } => {
                *has_return = true;
                match (function_return_type, value) {
                    (ValueType::Void, None) => {}
                    (ValueType::Void, Some(_)) => self
                        .diagnostics
                        .error(Some(*span), "Void function cannot return a value"),
                    (expected, None) => self.diagnostics.error(
                        Some(*span),
                        format!("Function must return '{}'", expected.as_str()),
                    ),
                    (expected, Some(expr)) => {
                        if let Some(actual) = self.analyze_expr(expr, locals, right) {
                            if !is_assignable(expected, &actual) {
                                self.diagnostics.error(
                                    Some(*span),
                                    format!(
                                        "Return type '{}' is incompatible with '{}'",
                                        actual.as_str(),
                                        expected.as_str()
                                    ),
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    fn analyze_local_var_decl(
        &mut self,
        decl: &VarDecl,
        locals: &mut HashMap<String, VariableInfo>,
        right: Option<&PermissionSet>,
    ) {
        if locals.contains_key(&decl.name) {
            self.diagnostics.error(
                Some(decl.span),
                format!("Local variable '{}' is already declared", decl.name),
            );
            return;
        }

        if let Some(capability) = &decl.entitlement {
            if !self.capabilities.contains_key(capability) {
                self.diagnostics.error(
                    Some(decl.span),
                    format!("Capability '{}' not found", capability),
                );
            }
        }

        let Some(initializer_type) = self.analyze_expr(&decl.initializer, locals, right) else {
            return;
        };

        let resolved_type = if let Some(explicit_type) = &decl.explicit_type {
            if !is_assignable(explicit_type, &initializer_type) {
                self.diagnostics.error(
                    Some(decl.span),
                    format!(
                        "Cannot initialize '{}' with value of type '{}'",
                        explicit_type.as_str(),
                        initializer_type.as_str()
                    ),
                );
                explicit_type.clone()
            } else {
                explicit_type.clone()
            }
        } else {
            initializer_type
        };

        locals.insert(
            decl.name.clone(),
            VariableInfo {
                ty: resolved_type,
                entitlement: decl.entitlement.clone(),
                span: decl.span,
            },
        );
    }

    fn analyze_expr(
        &mut self,
        expr: &Expr,
        locals: &HashMap<String, VariableInfo>,
        right: Option<&PermissionSet>,
    ) -> Option<ValueType> {
        match expr {
            Expr::IntLiteral(_, _) => Some(ValueType::Int),
            Expr::FloatLiteral(_, _) => Some(ValueType::Float),
            Expr::StringLiteral(text, span) => {
                let segments = match parse_segments(text) {
                    Ok(parts) => parts,
                    Err(message) => {
                        self.diagnostics.error(Some(*span), message);
                        return None;
                    }
                };

                for segment in segments {
                    if let Segment::Expression(code) = segment {
                        let parsed = match parse_expression_fragment(&code) {
                            Ok(expr) => expr,
                            Err(message) => {
                                self.diagnostics.error(
                                    Some(*span),
                                    format!("Invalid interpolation expression '{}': {}", code, message),
                                );
                                return None;
                            }
                        };

                        let Some(result_type) = self.analyze_expr(&parsed, locals, right) else {
                            self.diagnostics.error(
                                Some(*span),
                                format!("Interpolation expression '{}' is invalid", code),
                            );
                            return None;
                        };

                        match result_type {
                            ValueType::Int
                            | ValueType::Float
                            | ValueType::String
                            | ValueType::Bool => {}
                            _ => {
                                self.diagnostics.error(
                                    Some(*span),
                                    format!(
                                        "Interpolation does not support type '{}'",
                                        result_type.as_str()
                                    ),
                                );
                                return None;
                            }
                        }
                    }
                }

                Some(ValueType::String)
            }
            Expr::BoolLiteral(_, _) => Some(ValueType::Bool),
            Expr::ArrayLiteral(elements, span) => {
                if elements.is_empty() {
                    self.diagnostics.error(
                        Some(*span),
                        "Empty array literal requires explicit type annotation",
                    );
                    return None;
                }

                let mut element_type: Option<ValueType> = None;
                for element in elements {
                    let current = self.analyze_expr(element, locals, right)?;
                    element_type = match element_type {
                        None => Some(current),
                        Some(existing) if existing == current => Some(existing),
                        Some(existing) if is_numeric(&existing) && is_numeric(&current) => {
                            Some(ValueType::Float)
                        }
                        Some(existing) => {
                            self.diagnostics.error(
                                Some(*span),
                                format!(
                                    "Array literal has incompatible element types '{}' and '{}'",
                                    existing.as_str(),
                                    current.as_str()
                                ),
                            );
                            return None;
                        }
                    };
                }

                Some(ValueType::Array(Box::new(element_type?)))
            }
            Expr::Identifier(name, span) => {
                let variable = self.lookup_variable(name, locals)?;
                self.enforce_read_access(&variable, right, *span);
                Some(variable.ty)
            }
            Expr::Index { array, index, span } => {
                let array_type = self.analyze_expr(array, locals, right)?;
                let index_type = self.analyze_expr(index, locals, right)?;
                if index_type != ValueType::Int {
                    self.diagnostics.error(
                        Some(*span),
                        format!("Array index must be int, got '{}'", index_type.as_str()),
                    );
                    return None;
                }
                if let ValueType::Array(inner) = array_type {
                    Some((*inner).clone())
                } else {
                    self.diagnostics.error(
                        Some(*span),
                        format!(
                            "Indexing is only valid for arrays, got '{}'",
                            array_type.as_str()
                        ),
                    );
                    None
                }
            }
            Expr::NewObject { span, .. } => {
                self.diagnostics.error(
                    Some(*span),
                    "new Capability/new Right constructors are only allowed in capability/right declarations",
                );
                None
            }
            Expr::Unary { op, expr, span } => {
                let inner_type = self.analyze_expr(expr, locals, right)?;
                match op {
                    UnaryOp::Neg => {
                        if is_numeric(&inner_type) {
                            Some(inner_type)
                        } else {
                            self.diagnostics.error(
                                Some(*span),
                                format!(
                                    "Operator '-' is only valid for int/float, got '{}'",
                                    inner_type.as_str()
                                ),
                            );
                            return None;
                        }
                    }
                    UnaryOp::Not => {
                        if inner_type == ValueType::Bool {
                            Some(ValueType::Bool)
                        } else {
                            self.diagnostics.error(
                                Some(*span),
                                format!(
                                    "Operator '!' is only valid for bool, got '{}'",
                                    inner_type.as_str()
                                ),
                            );
                            None
                        }
                    }
                }
            }
            Expr::Binary {
                left,
                op,
                right: right_expr,
                span,
            } => {
                let left_type = self.analyze_expr(left, locals, right)?;
                let right_type = self.analyze_expr(right_expr, locals, right)?;
                self.analyze_binary_expr(op, &left_type, &right_type, *span)
            }
            Expr::Ternary {
                condition,
                then_expr,
                else_expr,
                span,
            } => {
                let condition_type = self.analyze_expr(condition, locals, right)?;
                if condition_type != ValueType::Bool {
                    self.diagnostics.error(
                        Some(condition.span()),
                        format!(
                            "Ternary condition must be bool, got '{}'",
                            condition_type.as_str()
                        ),
                    );
                    return None;
                }

                let then_type = self.analyze_expr(then_expr, locals, right)?;
                let else_type = self.analyze_expr(else_expr, locals, right)?;
                if then_type == else_type {
                    Some(then_type)
                } else if is_numeric(&then_type) && is_numeric(&else_type) {
                    Some(ValueType::Float)
                } else {
                    self.diagnostics.error(
                        Some(*span),
                        format!(
                            "Ternary branches have incompatible types '{}' and '{}'",
                            then_type.as_str(),
                            else_type.as_str()
                        ),
                    );
                    None
                }
            }
            Expr::Call { callee, args, span } => match callee {
                CallTarget::Name(function_name) => {
                    if function_name == "Int" {
                        if args.len() != 1 {
                            self.diagnostics.error(
                                Some(*span),
                                format!(
                                    "Int(...) expects exactly one argument, got {}",
                                    args.len()
                                ),
                            );
                            return None;
                        }
                        let arg_type = self.analyze_expr(&args[0], locals, right)?;
                        match arg_type {
                            ValueType::Int
                            | ValueType::Float
                            | ValueType::Bool
                            | ValueType::String => Some(ValueType::Int),
                            _ => {
                                self.diagnostics.error(
                                    Some(*span),
                                    format!(
                                        "Int(...) does not support conversion from '{}'",
                                        arg_type.as_str()
                                    ),
                                );
                                None
                            }
                        }
                    } else {
                        let Some(signature) = self.functions.get(function_name).cloned() else {
                            self.diagnostics.error(
                                Some(*span),
                                format!("Function '{}' not found", function_name),
                            );
                            return None;
                        };
                        if signature.params.len() != args.len() {
                            self.diagnostics.error(
                                Some(*span),
                                format!(
                                    "Function '{}' expects {} argument(s), got {}",
                                    function_name,
                                    signature.params.len(),
                                    args.len()
                                ),
                            );
                            return None;
                        }

                        for (index, argument) in args.iter().enumerate() {
                            let Some(argument_type) = self.analyze_expr(argument, locals, right)
                            else {
                                return None;
                            };
                            let expected = &signature.params[index];
                            if !is_assignable(expected, &argument_type) {
                                self.diagnostics.error(
                                    Some(argument.span()),
                                    format!(
                                        "Argument {} for '{}' has type '{}', expected '{}'",
                                        index + 1,
                                        function_name,
                                        argument_type.as_str(),
                                        expected.as_str()
                                    ),
                                );
                                return None;
                            }
                        }

                        Some(signature.return_type)
                    }
                }
                CallTarget::Qualified { module, name } => {
                    if !self.modules.contains(module) {
                        self.diagnostics
                            .error(Some(*span), format!("Module '{}' is not imported", module));
                        return None;
                    }

                    if module == "Bridge" {
                        if name == "println" || name == "print" {
                            for argument in args {
                                let argument_type = self.analyze_expr(argument, locals, right)?;
                                match argument_type {
                                    ValueType::Int
                                    | ValueType::Float
                                    | ValueType::String
                                    | ValueType::Bool => {}
                                    other => {
                                        self.diagnostics.error(
                                            Some(argument.span()),
                                            format!(
                                                "Bridge.{} does not support argument type '{}'",
                                                name,
                                                other.as_str()
                                            ),
                                        );
                                        return None;
                                    }
                                }
                            }
                            return Some(ValueType::Void);
                        }

                        if name == "sleep" || name == "usleep" {
                            if args.len() != 1 {
                                self.diagnostics.error(
                                    Some(*span),
                                    format!("Bridge.{} expects exactly one int argument", name),
                                );
                                return None;
                            }
                            let arg_ty = self.analyze_expr(&args[0], locals, right)?;
                            if arg_ty != ValueType::Int {
                                self.diagnostics.error(
                                    Some(args[0].span()),
                                    format!(
                                        "Bridge.{} expects int, got '{}'",
                                        name,
                                        arg_ty.as_str()
                                    ),
                                );
                                return None;
                            }
                            return Some(ValueType::Void);
                        }

                        if name == "system" {
                            for argument in args {
                                let arg_ty = self.analyze_expr(argument, locals, right)?;
                                if arg_ty != ValueType::String {
                                    self.diagnostics.error(
                                        Some(argument.span()),
                                        format!(
                                            "Bridge.system expects only string arguments, got '{}'",
                                            arg_ty.as_str()
                                        ),
                                    );
                                    return None;
                                }
                            }
                            return Some(ValueType::Void);
                        }
                    }

                    let mut argument_types = Vec::with_capacity(args.len());
                    for argument in args {
                        let argument_type = self.analyze_expr(argument, locals, right)?;
                        argument_types.push(argument_type);
                    }

                    if let Some(resolved) = self.modules.resolve_call(module, name, &argument_types)
                    {
                        Some(resolved.return_type)
                    } else {
                        let mut message = format!(
                            "No matching signature found for call {}.{}({})",
                            module,
                            name,
                            argument_types
                                .iter()
                                .map(ValueType::as_str)
                                .collect::<Vec<_>>()
                                .join(", ")
                        );
                        let expected = self.modules.expected_signatures(module, name);
                        if !expected.is_empty() {
                            message.push_str(&format!(". Available: {}", expected.join("; ")));
                        }
                        self.diagnostics.error(Some(*span), message);
                        None
                    }
                }
            },
        }
    }

    fn analyze_binary_expr(
        &mut self,
        op: &BinaryOp,
        left_type: &ValueType,
        right_type: &ValueType,
        span: Span,
    ) -> Option<ValueType> {
        match op {
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod => {
                if is_numeric(left_type) && is_numeric(right_type) {
                    if left_type == &ValueType::Float || right_type == &ValueType::Float {
                        Some(ValueType::Float)
                    } else {
                        Some(ValueType::Int)
                    }
                } else {
                    self.diagnostics.error(
                        Some(span),
                        format!(
                            "Arithmetic operator requires int/float, got '{}' and '{}'",
                            left_type.as_str(),
                            right_type.as_str()
                        ),
                    );
                    None
                }
            }
            BinaryOp::Eq | BinaryOp::NotEq => {
                if left_type == right_type || (is_numeric(left_type) && is_numeric(right_type)) {
                    Some(ValueType::Bool)
                } else {
                    self.diagnostics.error(
                        Some(span),
                        format!(
                            "Equality comparison requires compatible types, got '{}' and '{}'",
                            left_type.as_str(),
                            right_type.as_str()
                        ),
                    );
                    None
                }
            }
            BinaryOp::Greater | BinaryOp::GreaterEq | BinaryOp::Less | BinaryOp::LessEq => {
                if is_numeric(left_type) && is_numeric(right_type) {
                    Some(ValueType::Bool)
                } else {
                    self.diagnostics.error(
                        Some(span),
                        format!(
                            "Comparison operator requires int/float, got '{}' and '{}'",
                            left_type.as_str(),
                            right_type.as_str()
                        ),
                    );
                    None
                }
            }
            BinaryOp::And | BinaryOp::Or => {
                if left_type == &ValueType::Bool && right_type == &ValueType::Bool {
                    Some(ValueType::Bool)
                } else {
                    self.diagnostics.error(
                        Some(span),
                        format!(
                            "Logical operator requires bool, got '{}' and '{}'",
                            left_type.as_str(),
                            right_type.as_str()
                        ),
                    );
                    None
                }
            }
        }
    }

    fn lookup_variable(
        &mut self,
        name: &str,
        locals: &HashMap<String, VariableInfo>,
    ) -> Option<VariableInfo> {
        if let Some(local) = locals.get(name) {
            return Some(local.clone());
        }

        if let Some(global) = self.globals.get(name) {
            if global.ty == ValueType::Void {
                self.diagnostics.error(
                    Some(global.span),
                    format!(
                        "Global variable '{}' is used before type is resolved (check declaration order)",
                        name
                    ),
                );
                return None;
            }
            return Some(global.clone());
        }

        None
    }

    fn enforce_read_access(
        &mut self,
        variable: &VariableInfo,
        right: Option<&PermissionSet>,
        span: Span,
    ) {
        self.enforce_right(right, Operation::Read, span);
        self.enforce_capability(variable, Operation::Read, span);
    }

    fn enforce_write_access(
        &mut self,
        variable: &VariableInfo,
        right: Option<&PermissionSet>,
        span: Span,
    ) {
        self.enforce_right(right, Operation::Write, span);
        self.enforce_capability(variable, Operation::Write, span);
    }

    fn enforce_right(&mut self, right: Option<&PermissionSet>, operation: Operation, span: Span) {
        if let Some(right) = right {
            if !right.allows(operation) {
                self.diagnostics.error(
                    Some(span),
                    format!(
                        "Right violation: operation '{}' is not allowed (rule: {})",
                        op_name(operation),
                        right.raw.join(", ")
                    ),
                );
            }
        }
    }

    fn enforce_capability(&mut self, variable: &VariableInfo, operation: Operation, span: Span) {
        let Some(capability_name) = &variable.entitlement else {
            return;
        };

        if let Some(set) = self.capabilities.get(capability_name) {
            if !set.allows(operation) {
                self.diagnostics.error(
                    Some(span),
                    format!(
                        "Capability violation '{}': operation '{}' is not allowed (rule: {})",
                        capability_name,
                        op_name(operation),
                        set.raw.join(", ")
                    ),
                );
            }
        }
    }

    fn assignment_target_info(
        &mut self,
        target: &AssignTarget,
        locals: &HashMap<String, VariableInfo>,
        right: Option<&PermissionSet>,
        span: Span,
    ) -> Option<(VariableInfo, ValueType)> {
        match target {
            AssignTarget::Identifier(name) => {
                let variable = self.lookup_variable(name, locals)?;
                Some((variable.clone(), variable.ty))
            }
            AssignTarget::Index { array, index } => {
                let array_type = self.analyze_expr(array, locals, right)?;
                let index_type = self.analyze_expr(index, locals, right)?;
                if index_type != ValueType::Int {
                    self.diagnostics.error(
                        Some(span),
                        format!("Array index must be int, got '{}'", index_type.as_str()),
                    );
                    return None;
                }

                let owning_variable = self.find_root_variable(array, locals).ok_or_else(|| {
                    self.diagnostics.error(
                        Some(span),
                        "Array assignment target must originate from a variable",
                    );
                });
                let Ok(owning_variable) = owning_variable else {
                    return None;
                };

                if let ValueType::Array(inner) = array_type {
                    Some((owning_variable, (*inner).clone()))
                } else {
                    self.diagnostics.error(
                        Some(span),
                        format!(
                            "Index assignment requires array target, got '{}'",
                            array_type.as_str()
                        ),
                    );
                    None
                }
            }
        }
    }

    fn find_root_variable(
        &mut self,
        expr: &Expr,
        locals: &HashMap<String, VariableInfo>,
    ) -> Option<VariableInfo> {
        match expr {
            Expr::Identifier(name, _) => self.lookup_variable(name, locals),
            Expr::Index { array, .. } => self.find_root_variable(array, locals),
            _ => None,
        }
    }
}

fn is_numeric(ty: &ValueType) -> bool {
    matches!(ty, ValueType::Int | ValueType::Float)
}

fn is_assignable(target: &ValueType, source: &ValueType) -> bool {
    if target == source {
        return true;
    }

    if target == &ValueType::Float && source == &ValueType::Int {
        return true;
    }

    matches!(
        (target, source),
        (ValueType::Array(target_inner), ValueType::Array(source_inner))
            if is_assignable(target_inner, source_inner)
    )
}

fn op_name(operation: Operation) -> &'static str {
    match operation {
        Operation::Read => "read",
        Operation::Write => "write",
    }
}
