use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequirementArgArity {
    None,
    Optional,
    Required,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequirementSignature {
    pub name: &'static str,
    pub arg_arity: RequirementArgArity,
}

pub const REQUIREMENT_SIGNATURES: &[RequirementSignature] = &[
    RequirementSignature {
        name: "object.exists",
        arg_arity: RequirementArgArity::None,
    },
    RequirementSignature {
        name: "object.held",
        arg_arity: RequirementArgArity::None,
    },
    RequirementSignature {
        name: "object.not_held",
        arg_arity: RequirementArgArity::None,
    },
    RequirementSignature {
        name: "object.held_by_self",
        arg_arity: RequirementArgArity::None,
    },
    RequirementSignature {
        name: "object.held_by_other",
        arg_arity: RequirementArgArity::None,
    },
    RequirementSignature {
        name: "object.open",
        arg_arity: RequirementArgArity::None,
    },
    RequirementSignature {
        name: "object.closed",
        arg_arity: RequirementArgArity::None,
    },
    RequirementSignature {
        name: "object.opened_by_self",
        arg_arity: RequirementArgArity::None,
    },
    RequirementSignature {
        name: "object.opened_by_other",
        arg_arity: RequirementArgArity::None,
    },
    RequirementSignature {
        name: "world.owned",
        arg_arity: RequirementArgArity::None,
    },
    RequirementSignature {
        name: "room.in",
        arg_arity: RequirementArgArity::Optional,
    },
    RequirementSignature {
        name: "avatar.present",
        arg_arity: RequirementArgArity::None,
    },
];

const CONTRADICTIONS: &[(&str, &str)] = &[
    ("object.held", "object.not_held"),
    ("object.open", "object.closed"),
    ("object.held_by_self", "object.held_by_other"),
    ("object.opened_by_self", "object.opened_by_other"),
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RequirementValue {
    String(String),
    Bool(bool),
    Null,
}

impl RequirementValue {
    fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(value) => Some(*value),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyRequirement {
    pub name: String,
    pub arg: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RequirementKind {
    Legacy(LegacyRequirement),
    Expr(Expr),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequirementSpec {
    raw: String,
    kind: RequirementKind,
}

impl RequirementSpec {
    pub fn parse(input: &str) -> Result<Self, String> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err("empty requirement".to_string());
        }

        if let Ok(legacy) = parse_legacy_requirement(trimmed) {
            return Ok(Self {
                raw: trimmed.to_string(),
                kind: RequirementKind::Legacy(legacy),
            });
        }

        let expr = parse_expression(trimmed)?;
        Ok(Self {
            raw: trimmed.to_string(),
            kind: RequirementKind::Expr(expr),
        })
    }

    pub fn render(&self) -> String {
        self.raw.clone()
    }

    pub fn references_symbol(&self, symbol: &str) -> bool {
        match &self.kind {
            RequirementKind::Legacy(_) => false,
            RequirementKind::Expr(expr) => expr.references_symbol(symbol),
        }
    }

    fn as_legacy(&self) -> Option<&LegacyRequirement> {
        match &self.kind {
            RequirementKind::Legacy(legacy) => Some(legacy),
            RequirementKind::Expr(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequirementValidationIssueKind {
    UnknownRequirement,
    InvalidArgumentArity,
    Contradiction,
    Duplicate,
    UnknownSymbol,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequirementValidationIssue {
    pub kind: RequirementValidationIssueKind,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RequirementValidationReport {
    pub issues: Vec<RequirementValidationIssue>,
}

impl RequirementValidationReport {
    pub fn is_ok(&self) -> bool {
        self.issues.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequirementSet {
    pub all_of: Vec<RequirementSpec>,
}

impl RequirementSet {
    pub fn parse_many(items: &[String]) -> Result<Self, String> {
        let mut all_of = Vec::with_capacity(items.len());
        for item in items {
            all_of.push(RequirementSpec::parse(item)?);
        }
        Ok(Self { all_of })
    }

    pub fn validate(&self) -> RequirementValidationReport {
        validate_requirements(&self.all_of)
    }

    pub fn evaluate<C: RequirementChecker>(&self, checker: &C) -> RequirementEvaluation {
        evaluate_requirements(checker, &self.all_of)
    }
}

pub trait RequirementChecker {
    fn resolve_symbol(&self, symbol: &str) -> Option<RequirementValue>;

    fn check_legacy_requirement(&self, _requirement: &LegacyRequirement) -> bool {
        false
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RequirementEvaluation {
    pub passed: bool,
    pub failed: Vec<RequirementSpec>,
}

pub fn requirement_catalog() -> Vec<&'static str> {
    REQUIREMENT_SIGNATURES.iter().map(|sig| sig.name).collect()
}

pub fn validate_requirements(requirements: &[RequirementSpec]) -> RequirementValidationReport {
    let mut issues = Vec::new();

    for requirement in requirements {
        match &requirement.kind {
            RequirementKind::Legacy(legacy) => {
                let Some(signature) = REQUIREMENT_SIGNATURES
                    .iter()
                    .find(|sig| sig.name == legacy.name)
                    .copied()
                else {
                    issues.push(RequirementValidationIssue {
                        kind: RequirementValidationIssueKind::UnknownRequirement,
                        message: format!("unknown requirement '{}'", requirement.render()),
                    });
                    continue;
                };

                let arg_ok = match (signature.arg_arity, legacy.arg.as_ref()) {
                    (RequirementArgArity::None, None) => true,
                    (RequirementArgArity::None, Some(_)) => false,
                    (RequirementArgArity::Optional, _) => true,
                    (RequirementArgArity::Required, Some(arg)) => !arg.trim().is_empty(),
                    (RequirementArgArity::Required, None) => false,
                };

                if !arg_ok {
                    let arity_hint = match signature.arg_arity {
                        RequirementArgArity::None => "takes no argument",
                        RequirementArgArity::Optional => "takes an optional argument",
                        RequirementArgArity::Required => "requires an argument",
                    };
                    issues.push(RequirementValidationIssue {
                        kind: RequirementValidationIssueKind::InvalidArgumentArity,
                        message: format!(
                            "invalid requirement '{}': {}",
                            requirement.render(),
                            arity_hint
                        ),
                    });
                }
            }
            RequirementKind::Expr(expr) => {
                for symbol in expr.symbols() {
                    if !is_allowed_symbol(symbol.as_str()) {
                        issues.push(RequirementValidationIssue {
                            kind: RequirementValidationIssueKind::UnknownSymbol,
                            message: format!(
                                "unknown symbol '{}' in requirement '{}'",
                                symbol,
                                requirement.render()
                            ),
                        });
                    }
                }
            }
        }
    }

    for i in 0..requirements.len() {
        for j in (i + 1)..requirements.len() {
            let Some(a) = requirements[i].as_legacy() else {
                continue;
            };
            let Some(b) = requirements[j].as_legacy() else {
                continue;
            };

            if a.name == b.name && a.arg == b.arg {
                issues.push(RequirementValidationIssue {
                    kind: RequirementValidationIssueKind::Duplicate,
                    message: format!("duplicate requirement '{}'", requirements[i].render()),
                });
            }

            if CONTRADICTIONS
                .iter()
                .any(|(x, y)| (a.name == *x && b.name == *y) || (a.name == *y && b.name == *x))
            {
                issues.push(RequirementValidationIssue {
                    kind: RequirementValidationIssueKind::Contradiction,
                    message: format!(
                        "contradictory requirements '{}' and '{}'",
                        requirements[i].render(),
                        requirements[j].render()
                    ),
                });
            }
        }
    }

    RequirementValidationReport { issues }
}

pub fn evaluate_requirements<C: RequirementChecker>(
    checker: &C,
    requirements: &[RequirementSpec],
) -> RequirementEvaluation {
    let mut failed = Vec::new();

    for requirement in requirements {
        let passed = match &requirement.kind {
            RequirementKind::Legacy(legacy) => checker.check_legacy_requirement(legacy),
            RequirementKind::Expr(expr) => evaluate_expression(expr, checker).unwrap_or(false),
        };

        if !passed {
            failed.push(requirement.clone());
        }
    }

    RequirementEvaluation {
        passed: failed.is_empty(),
        failed,
    }
}

fn parse_legacy_requirement(input: &str) -> Result<LegacyRequirement, String> {
    if let Some(open_idx) = input.find('(') {
        if !input.ends_with(')') {
            return Err(format!(
                "invalid requirement '{}': missing closing ')'",
                input
            ));
        }
        let name = input[..open_idx].trim();
        let arg_raw = &input[open_idx + 1..input.len() - 1];
        let arg = arg_raw.trim();
        if arg.is_empty() {
            return Err(format!("invalid requirement '{}': empty argument", input));
        }
        validate_legacy_name(name)?;
        return Ok(LegacyRequirement {
            name: name.to_string(),
            arg: Some(arg.to_string()),
        });
    }

    validate_legacy_name(input)?;
    Ok(LegacyRequirement {
        name: input.to_string(),
        arg: None,
    })
}

fn validate_legacy_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("requirement name cannot be empty".to_string());
    }

    let valid = name
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '.' || ch == '_');
    if !valid {
        return Err(format!(
            "invalid requirement name '{}': only [a-z0-9._] allowed",
            name
        ));
    }

    if !name.contains('.') {
        return Err(format!(
            "invalid requirement name '{}': expected dotted form like domain.subject",
            name
        ));
    }

    Ok(())
}

fn is_allowed_symbol(symbol: &str) -> bool {
    matches!(
        symbol,
        "user" | "owner" | "location" | "opened_by" | "world.owner" | "world.slug"
    ) || symbol == "inbox"
        || (symbol.starts_with("room.")
            && symbol.ends_with(".inbox")
            && symbol.len() > "room..inbox".len())
        || symbol.starts_with("state.")
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Operand {
    Symbol(String),
    Literal(RequirementValue),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Expr {
    BoolLiteral(bool),
    SymbolTruthy(String),
    Eq(Operand, Operand),
    Ne(Operand, Operand),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    Not(Box<Expr>),
}

impl Expr {
    fn symbols(&self) -> Vec<String> {
        let mut out = Vec::new();
        self.collect_symbols(&mut out);
        out
    }

    fn references_symbol(&self, symbol: &str) -> bool {
        self.symbols().iter().any(|value| value == symbol)
    }

    fn collect_symbols(&self, out: &mut Vec<String>) {
        match self {
            Expr::BoolLiteral(_) => {}
            Expr::SymbolTruthy(name) => out.push(name.clone()),
            Expr::Eq(left, right) | Expr::Ne(left, right) => {
                collect_operand_symbol(left, out);
                collect_operand_symbol(right, out);
            }
            Expr::And(left, right) | Expr::Or(left, right) => {
                left.collect_symbols(out);
                right.collect_symbols(out);
            }
            Expr::Not(inner) => inner.collect_symbols(out),
        }
    }
}

fn collect_operand_symbol(operand: &Operand, out: &mut Vec<String>) {
    if let Operand::Symbol(name) = operand {
        out.push(name.clone());
    }
}

fn evaluate_expression<C: RequirementChecker>(expr: &Expr, checker: &C) -> Result<bool, String> {
    match expr {
        Expr::BoolLiteral(value) => Ok(*value),
        Expr::SymbolTruthy(symbol) => Ok(checker
            .resolve_symbol(symbol)
            .and_then(|value| value.as_bool())
            .unwrap_or(false)),
        Expr::Eq(left, right) => {
            Ok(resolve_operand_value(left, checker) == resolve_operand_value(right, checker))
        }
        Expr::Ne(left, right) => {
            Ok(resolve_operand_value(left, checker) != resolve_operand_value(right, checker))
        }
        Expr::And(left, right) => {
            Ok(evaluate_expression(left, checker)? && evaluate_expression(right, checker)?)
        }
        Expr::Or(left, right) => {
            Ok(evaluate_expression(left, checker)? || evaluate_expression(right, checker)?)
        }
        Expr::Not(inner) => Ok(!evaluate_expression(inner, checker)?),
    }
}

fn resolve_operand_value<C: RequirementChecker>(
    operand: &Operand,
    checker: &C,
) -> RequirementValue {
    match operand {
        Operand::Literal(value) => value.clone(),
        Operand::Symbol(symbol) => checker
            .resolve_symbol(symbol)
            .unwrap_or(RequirementValue::Null),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Ident(String),
    StringLiteral(String),
    Bool(bool),
    Null,
    EqEq,
    NotEq,
    AndAnd,
    OrOr,
    Not,
    LParen,
    RParen,
}

fn parse_expression(input: &str) -> Result<Expr, String> {
    let tokens = tokenize(input)?;
    if tokens.is_empty() {
        return Err("empty expression".to_string());
    }
    let mut parser = Parser { tokens, index: 0 };
    let expr = parser.parse_or()?;
    if parser.index != parser.tokens.len() {
        return Err("unexpected trailing tokens in expression".to_string());
    }
    Ok(expr)
}

fn tokenize(input: &str) -> Result<Vec<Token>, String> {
    let chars: Vec<char> = input.chars().collect();
    let mut idx = 0;
    let mut out = Vec::new();

    while idx < chars.len() {
        let ch = chars[idx];
        if ch.is_whitespace() {
            idx += 1;
            continue;
        }

        if ch == '(' {
            out.push(Token::LParen);
            idx += 1;
            continue;
        }
        if ch == ')' {
            out.push(Token::RParen);
            idx += 1;
            continue;
        }
        if ch == '!' {
            if idx + 1 < chars.len() && chars[idx + 1] == '=' {
                out.push(Token::NotEq);
                idx += 2;
            } else {
                out.push(Token::Not);
                idx += 1;
            }
            continue;
        }
        if ch == '=' {
            if idx + 1 < chars.len() && chars[idx + 1] == '=' {
                out.push(Token::EqEq);
                idx += 2;
                continue;
            }
            return Err("single '=' is not allowed; use '=='".to_string());
        }
        if ch == '&' {
            if idx + 1 < chars.len() && chars[idx + 1] == '&' {
                out.push(Token::AndAnd);
                idx += 2;
                continue;
            }
            return Err("single '&' is not allowed; use '&&'".to_string());
        }
        if ch == '|' {
            if idx + 1 < chars.len() && chars[idx + 1] == '|' {
                out.push(Token::OrOr);
                idx += 2;
                continue;
            }
            return Err("single '|' is not allowed; use '||'".to_string());
        }
        if ch == '\'' || ch == '"' {
            let quote = ch;
            idx += 1;
            let start = idx;
            while idx < chars.len() && chars[idx] != quote {
                idx += 1;
            }
            if idx >= chars.len() {
                return Err("unterminated string literal".to_string());
            }
            let value: String = chars[start..idx].iter().collect();
            out.push(Token::StringLiteral(value));
            idx += 1;
            continue;
        }
        if ch.is_ascii_alphabetic() || ch == '_' {
            let start = idx;
            idx += 1;
            while idx < chars.len()
                && (chars[idx].is_ascii_alphanumeric() || chars[idx] == '_' || chars[idx] == '.')
            {
                idx += 1;
            }
            let token: String = chars[start..idx].iter().collect();
            match token.as_str() {
                "true" => out.push(Token::Bool(true)),
                "false" => out.push(Token::Bool(false)),
                "null" => out.push(Token::Null),
                _ => out.push(Token::Ident(token)),
            }
            continue;
        }

        return Err(format!("unexpected character '{}' in expression", ch));
    }

    Ok(out)
}

struct Parser {
    tokens: Vec<Token>,
    index: usize,
}

impl Parser {
    fn parse_or(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_and()?;
        while self.match_token(&Token::OrOr) {
            let right = self.parse_and()?;
            left = Expr::Or(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_unary()?;
        while self.match_token(&Token::AndAnd) {
            let right = self.parse_unary()?;
            left = Expr::And(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expr, String> {
        if self.match_token(&Token::Not) {
            let inner = self.parse_unary()?;
            return Ok(Expr::Not(Box::new(inner)));
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<Expr, String> {
        if self.match_token(&Token::LParen) {
            let expr = self.parse_or()?;
            self.expect_token(&Token::RParen)?;
            return Ok(expr);
        }

        let left = self.parse_operand()?;
        if self.match_token(&Token::EqEq) {
            let right = self.parse_operand()?;
            return Ok(Expr::Eq(left, right));
        }
        if self.match_token(&Token::NotEq) {
            let right = self.parse_operand()?;
            return Ok(Expr::Ne(left, right));
        }

        match left {
            Operand::Literal(RequirementValue::Bool(value)) => Ok(Expr::BoolLiteral(value)),
            Operand::Symbol(name) => Ok(Expr::SymbolTruthy(name)),
            _ => Err("expected comparison operator '==' or '!='".to_string()),
        }
    }

    fn parse_operand(&mut self) -> Result<Operand, String> {
        let Some(token) = self.peek().cloned() else {
            return Err("expected operand".to_string());
        };

        self.index += 1;
        match token {
            Token::Ident(name) => Ok(Operand::Symbol(name)),
            Token::StringLiteral(value) => Ok(Operand::Literal(RequirementValue::String(value))),
            Token::Bool(value) => Ok(Operand::Literal(RequirementValue::Bool(value))),
            Token::Null => Ok(Operand::Literal(RequirementValue::Null)),
            _ => Err("expected symbol or literal".to_string()),
        }
    }

    fn expect_token(&mut self, expected: &Token) -> Result<(), String> {
        if self.match_token(expected) {
            Ok(())
        } else {
            Err("unexpected token in expression".to_string())
        }
    }

    fn match_token(&mut self, expected: &Token) -> bool {
        if self.peek() == Some(expected) {
            self.index += 1;
            true
        } else {
            false
        }
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct DummyChecker;

    impl RequirementChecker for DummyChecker {
        fn resolve_symbol(&self, symbol: &str) -> Option<RequirementValue> {
            match symbol {
                "user" => Some(RequirementValue::String("did:ma:user".to_string())),
                "owner" => Some(RequirementValue::String("did:ma:user".to_string())),
                "location" => Some(RequirementValue::String("did:ma:user".to_string())),
                "state.open" => Some(RequirementValue::Bool(true)),
                _ => None,
            }
        }

        fn check_legacy_requirement(&self, requirement: &LegacyRequirement) -> bool {
            requirement.name == "object.held"
        }
    }

    #[test]
    fn parses_and_evaluates_symbol_expression() {
        let req = RequirementSpec::parse("user == owner && user == location").expect("parse ok");
        let out = evaluate_requirements(&DummyChecker, &[req]);
        assert!(out.passed);
    }

    #[test]
    fn validates_unknown_symbol() {
        let req = RequirementSpec::parse("user == admin").expect("parse ok");
        let report = validate_requirements(&[req]);
        assert!(!report.is_ok());
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.kind == RequirementValidationIssueKind::UnknownSymbol));
    }

    #[test]
    fn supports_legacy_requirements() {
        let req = RequirementSpec::parse("object.held").expect("legacy parse");
        let out = evaluate_requirements(&DummyChecker, &[req]);
        assert!(out.passed);
    }
}
