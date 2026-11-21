//! Equivalence for the terminating scalar fragment of raw, typed Mesh MIR.

use mesh_codegen::mir::{
    BinOp, MirExpr, MirFunction, MirLiteral, MirMatchArm, MirModule, MirPattern, MirSourceSpan,
    MirType, UnaryOp,
};
use serde_json::Value as Json;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::fmt::Write as _;
use std::process::Command;
use std::time::{Duration, Instant};

use crate::verify_process::{self, RunError};

const Z3_VERSION: &str = "4.15.7";
const MAX_EXPRESSIONS: usize = 20_000;
const MAX_DEPTH: usize = 96;

#[derive(Debug, PartialEq, serde::Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum Verdict {
    Verified,
    Falsified {
        inputs: BTreeMap<String, Json>,
        expected: Json,
        actual: Json,
    },
    Unverified {
        reason: String,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct EncodingError {
    pub(crate) reason: String,
    pub(crate) location: Option<MirSourceSpan>,
}

impl EncodingError {
    fn context(mut self, context: &str) -> Self {
        self.reason = format!("{context}: {}", self.reason);
        self
    }

    fn at(mut self, source: Option<MirSourceSpan>) -> Self {
        self.location = self.location.or(source);
        self
    }
}

impl From<String> for EncodingError {
    fn from(reason: String) -> Self {
        Self {
            reason,
            location: None,
        }
    }
}

impl From<&str> for EncodingError {
    fn from(reason: &str) -> Self {
        reason.to_string().into()
    }
}

impl From<mesh_diagnostics::Diagnostic> for EncodingError {
    fn from(error: mesh_diagnostics::Diagnostic) -> Self {
        crate::diagnostic_reason(&error).into()
    }
}

pub(crate) fn check(
    program: &[MirModule],
    target: &MirFunction,
    expected: &MirFunction,
) -> Result<Verdict, EncodingError> {
    let query = Query::encode(program, target, expected)?;
    Ok(
        match Solver::configured().and_then(|solver| solver.solve(&query)) {
            Ok(verdict) => verdict,
            Err(reason) => Verdict::Unverified { reason },
        },
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sort {
    Int,
    Bool,
}

impl Sort {
    fn from_mir(ty: &MirType) -> Result<Self, String> {
        match ty {
            MirType::Int => Ok(Self::Int),
            MirType::Bool => Ok(Self::Bool),
            _ => Err(format!("unsupported scalar type {ty}")),
        }
    }

    fn smt(self) -> &'static str {
        match self {
            Self::Int => "(_ BitVec 64)",
            Self::Bool => "Bool",
        }
    }
}

#[derive(Clone)]
struct Term {
    sort: Sort,
    smt: String,
}

impl Term {
    fn int(value: i64) -> Self {
        Self {
            sort: Sort::Int,
            smt: format!("#x{:016x}", value as u64),
        }
    }

    fn boolean(value: bool) -> Self {
        Self {
            sort: Sort::Bool,
            smt: value.to_string(),
        }
    }
}

#[derive(Clone)]
enum Value {
    Scalar(Term),
    Unit,
}

impl Value {
    fn scalar(self) -> Result<Term, String> {
        match self {
            Self::Scalar(term) => Ok(term),
            Self::Unit => Err("expected an Int or Bool expression, got Unit".into()),
        }
    }

    fn require(&self, ty: &MirType) -> Result<(), String> {
        match (self, ty) {
            (Self::Scalar(term), _) if Sort::from_mir(ty)? == term.sort => Ok(()),
            (Self::Unit, MirType::Unit) => Ok(()),
            _ => Err(format!(
                "MIR expression does not have its declared type {ty}"
            )),
        }
    }
}

type Locals<'a> = Vec<(&'a str, Value)>;

struct Input<'a> {
    name: &'a str,
    sort: Sort,
}

struct Query<'a> {
    assertions: String,
    inputs: Vec<Input<'a>>,
    result_sort: Sort,
}

impl<'a> Query<'a> {
    fn encode(
        program: &'a [MirModule],
        target: &'a MirFunction,
        expected: &'a MirFunction,
    ) -> Result<Self, EncodingError> {
        if target.return_type != expected.return_type {
            return Err("law and implementation result types differ".into());
        }
        let result_sort = Sort::from_mir(&target.return_type)?;
        if !target
            .params
            .iter()
            .map(|(_, ty)| ty)
            .eq(expected.params.iter().map(|(_, ty)| ty))
        {
            return Err("law and implementation parameter types differ".into());
        }

        let mut encoder = Encoder::new(program);
        if !std::ptr::eq(encoder.resolve(&target.name)?, target) {
            return Err(format!(
                "implementation symbol {} is not its selected declaration",
                target.name
            )
            .into());
        }

        let mut inputs = Vec::new();
        let mut arguments = Vec::new();
        let mut names = HashSet::new();
        for (index, (name, ty)) in expected.params.iter().enumerate() {
            if !names.insert(name) {
                return Err(format!("duplicate law parameter {name}").into());
            }
            let sort = Sort::from_mir(ty)?;
            let symbol = format!("input{index}");
            writeln!(encoder.smt, "(declare-const {symbol} {})", sort.smt()).unwrap();
            arguments.push(Value::Scalar(Term { sort, smt: symbol }));
            inputs.push(Input { name, sort });
        }

        let actual = encoder
            .function(target, &arguments, 0)
            .map_err(|error| error.context("implementation"))?
            .scalar()?;
        encoder.functions.clear();
        let expected = encoder
            .function(expected, &arguments, 0)
            .map_err(|error| error.context("expected expression"))?
            .scalar()?;
        let result_type = result_sort.smt();
        writeln!(
            encoder.smt,
            "(define-fun actual () {result_type} {})",
            actual.smt
        )
        .unwrap();
        writeln!(
            encoder.smt,
            "(define-fun expected () {result_type} {})",
            expected.smt
        )
        .unwrap();
        encoder
            .smt
            .push_str("(assert (distinct actual expected))\n(check-sat)\n");
        Ok(Self {
            assertions: encoder.smt,
            inputs,
            result_sort,
        })
    }

    fn value_request(&self) -> String {
        let mut request = "(get-value (actual expected".to_string();
        for index in 0..self.inputs.len() {
            write!(request, " input{index}").unwrap();
        }
        request.push_str("))\n");
        request
    }
}

struct Encoder<'a> {
    functions: HashMap<&'a str, Option<&'a MirFunction>>,
    active: Vec<&'a str>,
    remaining: usize,
    next_symbol: usize,
    smt: String,
}

impl<'a> Encoder<'a> {
    fn new(program: &'a [MirModule]) -> Self {
        let mut functions = HashMap::new();
        for module in program {
            for function in &module.functions {
                functions
                    .entry(function.name.as_str())
                    .and_modify(|declaration| *declaration = None)
                    .or_insert(Some(function));
            }
        }
        // A native declaration collision makes the proof target ambiguous.
        for native in program.iter().flat_map(|module| &module.native_functions) {
            for name in [&native.name, &native.symbol] {
                if let Some(declaration) = functions.get_mut(name.as_str()) {
                    *declaration = None;
                }
            }
        }
        Self {
            functions,
            active: Vec::new(),
            remaining: MAX_EXPRESSIONS,
            next_symbol: 0,
            smt: String::from("(set-logic QF_BV)\n"),
        }
    }

    fn resolve(&self, name: &str) -> Result<&'a MirFunction, String> {
        // Codegen recognizes runtime and builtin symbols before looking up user calls.
        if name.starts_with("mesh_") || name.starts_with("__mesh_") {
            return Err(format!(
                "unsupported call or target {name} (runtime or builtin symbol)"
            ));
        }
        match self.functions.get(name) {
            Some(Some(function)) => Ok(function),
            Some(None) => Err(format!("ambiguous MIR function symbol {name}")),
            None => Err(format!(
                "unsupported call or target {name} (no inspectable Mesh body)"
            )),
        }
    }

    fn function(
        &mut self,
        function: &'a MirFunction,
        arguments: &[Value],
        depth: usize,
    ) -> Result<Value, EncodingError> {
        let name = function.name.as_str();
        if self.active.contains(&name) {
            return Err(format!("unsupported recursion in {name}").into());
        }
        if function.is_closure_fn || !function.captures.is_empty() {
            return Err(format!("unsupported closure function {name}").into());
        }
        if arguments.len() != function.params.len() {
            return Err(format!("MIR argument count differs for {name}").into());
        }

        let mut locals = Vec::new();
        for ((parameter, ty), argument) in function.params.iter().zip(arguments) {
            argument.require(ty)?;
            locals.push((parameter.as_str(), argument.clone()));
        }

        self.active.push(name);
        let result = self
            .expression(&function.body, &mut locals, depth)
            .and_then(|value| {
                value.require(&function.return_type)?;
                Ok(value)
            });
        self.active.pop();
        result.map_err(|error| error.context(&format!("in {name}")))
    }

    fn emit(&mut self, sort: Sort, expression: impl std::fmt::Display) -> Term {
        let symbol = format!("v{}", self.next_symbol);
        self.next_symbol += 1;
        writeln!(
            self.smt,
            "(define-fun {symbol} () {} {expression})",
            sort.smt()
        )
        .unwrap();
        Term { sort, smt: symbol }
    }

    fn expression(
        &mut self,
        expr: &'a MirExpr,
        locals: &mut Locals<'a>,
        depth: usize,
    ) -> Result<Value, EncodingError> {
        self.encode_expression(expr, locals, depth)
            .map_err(|error| error.at(expr.source_span()))
    }

    fn encode_expression(
        &mut self,
        expr: &'a MirExpr,
        locals: &mut Locals<'a>,
        depth: usize,
    ) -> Result<Value, EncodingError> {
        if depth >= MAX_DEPTH || self.remaining == 0 {
            return Err("MIR encoding budget exhausted".into());
        }
        self.remaining -= 1;
        let next = depth + 1;
        let value = match expr {
            MirExpr::IntLit(value, _) => Value::Scalar(Term::int(*value)),
            MirExpr::BoolLit(value, _) => Value::Scalar(Term::boolean(*value)),
            MirExpr::Unit => Value::Unit,
            MirExpr::Var(name, _) => locals
                .iter()
                .rev()
                .find(|(local, _)| *local == name)
                .map(|(_, value)| value.clone())
                .ok_or_else(|| format!("unsupported non-local value {name}"))?,
            MirExpr::BinOp { op, lhs, rhs, .. } => {
                let left = self.expression(lhs, locals, next)?;
                let right = self.expression(rhs, locals, next)?;
                left.require(lhs.ty())?;
                right.require(rhs.ty())?;
                let lhs = left.scalar()?;
                let rhs = right.scalar()?;
                Value::Scalar(self.binary(*op, &lhs, &rhs)?)
            }
            MirExpr::UnaryOp { op, operand, .. } => {
                let operand = self.expression(operand, locals, next)?.scalar()?;
                let operator = match (op, operand.sort) {
                    (UnaryOp::Neg, Sort::Int) => "bvneg",
                    (UnaryOp::Not, Sort::Bool) => "not",
                    _ => return Err(format!("unsupported unary operator {op:?}").into()),
                };
                Value::Scalar(self.emit(operand.sort, format_args!("({operator} {})", operand.smt)))
            }
            MirExpr::If {
                cond,
                then_body,
                else_body,
                ..
            } => {
                let cond = self.expression(cond, locals, next)?.scalar()?;
                let yes = self.expression(then_body, locals, next)?;
                let no = self.expression(else_body, locals, next)?;
                self.select(&cond, yes, no)?
            }
            MirExpr::Let {
                name,
                ty,
                value,
                body,
            } => {
                let value = self.expression(value, locals, next)?;
                value.require(ty)?;
                locals.push((name, value));
                let result = self.expression(body, locals, next);
                locals.pop();
                result?
            }
            MirExpr::Block(expressions, _) => {
                let mut last = Value::Unit;
                for expression in expressions {
                    last = self.expression(expression, locals, next)?;
                }
                last
            }
            MirExpr::Match {
                scrutinee, arms, ..
            } => {
                let value = self.expression(scrutinee, locals, next)?;
                value.require(scrutinee.ty())?;
                self.match_arms(&value.scalar()?, arms, locals, next)?
            }
            MirExpr::Call { func, args, .. } => {
                let MirExpr::Var(name, _) = func.as_ref() else {
                    return Err("unsupported indirect call".into());
                };
                if locals.iter().any(|(local, _)| *local == name) {
                    return Err(format!("unsupported indirect call through {name}").into());
                }
                let callee = self.resolve(name)?;
                let arguments = args
                    .iter()
                    .map(|argument| self.expression(argument, locals, next))
                    .collect::<Result<Vec<_>, _>>()?;
                self.function(callee, &arguments, next)?
            }
            MirExpr::TailCall { .. } => return Err("unsupported recursion".into()),
            MirExpr::Panic { .. } => return Err("unsupported panic".into()),
            MirExpr::Return(..) => return Err("unsupported explicit return".into()),
            MirExpr::ClosureCall { .. } => return Err("unsupported closure call".into()),
            MirExpr::MakeClosure { .. } => return Err("unsupported closure value".into()),
            _ => return Err(format!("unsupported MIR expression with type {}", expr.ty()).into()),
        };
        // MirExpr::ty on a let is the binding's type, even when the
        // continuation has another type.
        if !matches!(expr, MirExpr::Let { .. }) {
            value.require(expr.ty())?;
        }
        Ok(value)
    }

    fn binary(&mut self, op: BinOp, lhs: &Term, rhs: &Term) -> Result<Term, String> {
        if matches!(op, BinOp::Div | BinOp::Mod) {
            return Err("unsupported integer division or remainder".into());
        }
        if lhs.sort != rhs.sort {
            return Err(format!("different MIR operand types for {op:?}"));
        }
        let (operator, result) = match (op, lhs.sort) {
            (BinOp::Eq, _) => ("=", Sort::Bool),
            (BinOp::NotEq, _) => ("distinct", Sort::Bool),
            (BinOp::And, Sort::Bool) => ("and", Sort::Bool),
            (BinOp::Or, Sort::Bool) => ("or", Sort::Bool),
            (BinOp::Add, Sort::Int) => ("bvadd", Sort::Int),
            (BinOp::Sub, Sort::Int) => ("bvsub", Sort::Int),
            (BinOp::Mul, Sort::Int) => ("bvmul", Sort::Int),
            (BinOp::Lt, Sort::Int) => ("bvslt", Sort::Bool),
            (BinOp::Gt, Sort::Int) => ("bvsgt", Sort::Bool),
            (BinOp::LtEq, Sort::Int) => ("bvsle", Sort::Bool),
            (BinOp::GtEq, Sort::Int) => ("bvsge", Sort::Bool),
            _ => return Err(format!("unsupported operator {op:?} for {:?}", lhs.sort)),
        };
        Ok(self.emit(result, format_args!("({operator} {} {})", lhs.smt, rhs.smt)))
    }

    fn select(&mut self, condition: &Term, yes: Value, no: Value) -> Result<Value, String> {
        if condition.sort != Sort::Bool {
            return Err("expected a Bool condition".into());
        }
        match (yes, no) {
            (Value::Unit, Value::Unit) => Ok(Value::Unit),
            (Value::Scalar(yes), Value::Scalar(no)) if yes.sort == no.sort => {
                Ok(Value::Scalar(self.emit(
                    yes.sort,
                    format_args!("(ite {} {} {})", condition.smt, yes.smt, no.smt),
                )))
            }
            _ => Err("MIR branch result types differ".into()),
        }
    }

    fn match_arms(
        &mut self,
        scrutinee: &Term,
        arms: &'a [MirMatchArm],
        locals: &mut Locals<'a>,
        depth: usize,
    ) -> Result<Value, EncodingError> {
        let mut encoded = Vec::new();
        for arm in arms {
            let locals_before = locals.len();
            let mut condition = match &arm.pattern {
                MirPattern::Wildcard => None,
                MirPattern::Var(name, ty) if Sort::from_mir(ty)? == scrutinee.sort => {
                    locals.push((name, Value::Scalar(scrutinee.clone())));
                    None
                }
                MirPattern::Literal(MirLiteral::Int(value)) => {
                    Some(self.binary(BinOp::Eq, scrutinee, &Term::int(*value))?)
                }
                MirPattern::Literal(MirLiteral::Bool(value)) => {
                    Some(self.binary(BinOp::Eq, scrutinee, &Term::boolean(*value))?)
                }
                _ => return Err("unsupported case pattern".into()),
            };
            if let Some(guard) = &arm.guard {
                let guard = self.expression(guard, locals, depth)?.scalar()?;
                if guard.sort != Sort::Bool {
                    return Err("expected a Bool case guard".into());
                }
                condition = Some(match condition {
                    Some(pattern) => self.binary(BinOp::And, &pattern, &guard)?,
                    None => guard,
                });
            }
            let value = self.expression(&arm.body, locals, depth)?;
            locals.truncate(locals_before);
            encoded.push((condition, value));
        }

        // Require syntactic coverage and visit even shadowed arms. When only
        // total, effect-free terms are admitted, SMT ite preserves first match.
        let Some((None, mut result)) = encoded.pop() else {
            return Err("unsupported case without a final unguarded catchall".into());
        };
        for (condition, value) in encoded.into_iter().rev() {
            result = match condition {
                Some(condition) => self.select(&condition, value, result)?,
                None => value,
            };
        }
        Ok(result)
    }
}

struct Solver {
    executable: OsString,
    deadline: Instant,
}

impl Solver {
    fn configured() -> Result<Self, String> {
        let executable = std::env::var_os("MESH_VERIFY_Z3").unwrap_or_else(|| "z3".into());
        Ok(Self::new(
            executable,
            verify_process::timeout_ms(verify_process::DEFAULT_TIMEOUT_MS)?,
        ))
    }

    fn new(executable: OsString, timeout_ms: u32) -> Self {
        Self {
            executable,
            deadline: Instant::now() + Duration::from_millis(u64::from(timeout_ms)),
        }
    }

    fn remaining(&self) -> Result<Duration, String> {
        self.deadline
            .checked_duration_since(Instant::now())
            .filter(|duration| !duration.is_zero())
            .ok_or_else(|| "solver time budget exhausted".to_string())
    }

    fn solve(&self, query: &Query<'_>) -> Result<Verdict, String> {
        let version = self.run(&["--version"], "")?;
        let mut words = version.split_whitespace();
        if (words.next(), words.next(), words.next())
            != (Some("Z3"), Some("version"), Some(Z3_VERSION))
        {
            return Err(format!(
                "mesh verify requires Z3 {Z3_VERSION}; found {}",
                version.trim()
            ));
        }

        let status = self.run_query(query, "(get-info :reason-unknown)\n")?;
        match solver_response(&status) {
            Some(("unsat", reason)) if reason.trim() == "(:reason-unknown \"\")" => {
                Ok(Verdict::Verified)
            }
            Some(("sat", reason)) if reason.trim() == "(:reason-unknown \"\")" => {
                // The SMT-LIB get-value command is valid only after sat.
                let witness = self.run_query(query, &query.value_request())?;
                match solver_response(&witness) {
                    Some(("sat", model)) => parse_witness(query, model),
                    Some(("unknown", _)) => {
                        Err("Z3 returned unknown while extracting a witness".into())
                    }
                    _ => Err(
                        "Z3 did not reproduce its satisfiable verdict for witness extraction"
                            .into(),
                    ),
                }
            }
            Some(("unknown", reason)) => Err(format!(
                "Z3 returned unknown ({})",
                reason
                    .trim()
                    .strip_prefix("(:reason-unknown \"")
                    .and_then(|reason| reason.strip_suffix("\")"))
                    .unwrap_or("reason unavailable"),
            )),
            _ => Err("unexpected Z3 check-sat response".into()),
        }
    }

    fn run_query(&self, query: &Query<'_>, follow_up: &str) -> Result<String, String> {
        let timeout_ms = self.remaining()?.as_millis().max(1);
        let input = format!(
            "(set-option :timeout {timeout_ms})\n{}{follow_up}",
            query.assertions,
        );
        self.run(&["-in", "-smt2"], &input)
    }

    fn run(&self, args: &[&str], input: &str) -> Result<String, String> {
        let output = verify_process::run(
            Command::new(&self.executable).args(args),
            input,
            self.deadline,
        )
        .map_err(|error| match error {
            RunError::Start(error) => executable_error(&self.executable, error),
            RunError::Io(error) => format!("Z3 I/O: {error}"),
            RunError::Timeout => "solver time budget exhausted".into(),
        })?;
        if !output.status.success() || !output.stderr.trim().is_empty() {
            let detail = if output.stderr.trim().is_empty() {
                &output.stdout
            } else {
                &output.stderr
            };
            let brief: String = detail.trim().chars().take(240).collect();
            return Err(format!("Z3 failed ({}): {brief}", output.status));
        }
        Ok(output.stdout)
    }
}

fn solver_response(output: &str) -> Option<(&str, &str)> {
    let (status, remainder) = output.split_once('\n')?;
    Some((status.trim_end_matches('\r'), remainder))
}

fn executable_error(executable: &OsStr, error: std::io::Error) -> String {
    format!(
        "could not start Z3 {Z3_VERSION} at {}: {error}; set MESH_VERIFY_Z3 to its executable",
        executable.to_string_lossy(),
    )
}

fn parse_witness(query: &Query<'_>, model: &str) -> Result<Verdict, String> {
    let tokens = model.replace('(', " ( ").replace(')', " ) ");
    let tokens: Vec<_> = tokens.split_whitespace().collect();
    let invalid = || "invalid or incomplete Z3 witness".to_string();
    let ["(", entries @ .., ")"] = tokens.as_slice() else {
        return Err(invalid());
    };
    let mut pairs = BTreeMap::new();
    let chunks = entries.chunks_exact(4);
    if !chunks.remainder().is_empty() || chunks.len() != query.inputs.len() + 2 {
        return Err(invalid());
    }
    for pair in chunks {
        let ["(", name, value, ")"] = pair else {
            return Err(invalid());
        };
        if pairs.insert(*name, *value).is_some() {
            return Err(invalid());
        }
    }

    let mut get = |name: &str, sort| {
        pairs
            .remove(name)
            .and_then(|value| parse_scalar(value, sort))
            .ok_or_else(invalid)
    };
    let actual = get("actual", query.result_sort)?;
    let expected = get("expected", query.result_sort)?;
    if actual == expected {
        return Err("Z3 witness gives equal actual and expected results".into());
    }

    let mut inputs = BTreeMap::new();
    for (index, input) in query.inputs.iter().enumerate() {
        inputs.insert(
            input.name.to_string(),
            get(&format!("input{index}"), input.sort)?,
        );
    }
    Ok(Verdict::Falsified {
        inputs,
        expected,
        actual,
    })
}

fn parse_scalar(value: &str, sort: Sort) -> Option<Json> {
    match sort {
        Sort::Bool => value.parse::<bool>().ok().map(Json::from),
        Sort::Int => {
            let (radix, digits) =
                if let Some(hex) = value.strip_prefix("#x").filter(|hex| hex.len() == 16) {
                    (16, hex)
                } else {
                    (2, value.strip_prefix("#b").filter(|bits| bits.len() == 64)?)
                };
            let bits = u64::from_str_radix(digits, radix).ok()?;
            Some(Json::from(bits as i64))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const POLICY: &str = r#"
fn eligible(code :: Int) -> Bool do
  code == 429 || code == 503
end

fn retry(code :: Int, budget :: Int) -> Bool do
  let allowed = eligible(code)
  if allowed do
    budget > 0
  else
    false
  end
end
"#;

    const LAW: &str = r#"
fn decision(status :: Int, left :: Int) -> Bool do
  case status do
    429 -> left > 0
    503 -> left > 0
    _ -> false
  end
end
"#;

    fn lower(source: &str, module: &str) -> MirModule {
        let parse = mesh_parser::parse(source);
        assert!(parse.errors().is_empty(), "{:?}", parse.errors());
        let checked = mesh_typeck::check(&parse);
        assert!(checked.errors.is_empty(), "{:?}", checked.errors);
        mesh_codegen::lower_module_to_mir_raw(
            &parse,
            &checked,
            module,
            &Default::default(),
            &Default::default(),
            &[],
        )
        .unwrap()
    }

    fn function<'a>(module: &'a MirModule, name: &str) -> &'a MirFunction {
        module
            .functions
            .iter()
            .find(|function| function.name == name)
            .unwrap()
    }

    fn query<'a>(program: &'a [MirModule], expected: &'a MirModule) -> Query<'a> {
        Query::encode(
            program,
            function(&program[0], "Policy__retry"),
            function(expected, "decision"),
        )
        .unwrap()
    }

    #[test]
    fn rejects_discarded_effect_on_a_short_circuited_path() {
        let policy = POLICY
            .replace(
                "code == 429 || code == 503",
                "let ignored = DateTime.utc_now()\n  code == 429 || code == 503",
            )
            .replace(
                "let allowed = eligible(code)",
                "let allowed = false && eligible(code)",
            );
        let program = [lower(&policy, "Policy")];
        let law = lower(LAW, "");
        let result = check(
            &program,
            function(&program[0], "Policy__retry"),
            function(&law, "decision"),
        );
        let Err(EncodingError { reason, .. }) = result else {
            panic!("discarded clock effect received {result:?}");
        };
        assert!(reason.contains("Policy__eligible"), "{reason}");
        assert!(reason.contains("mesh_datetime_utc_now"), "{reason}");
    }

    #[test]
    fn rejects_recursion_and_division_before_starting_z3() {
        let law = lower(LAW, "");
        for (body, unsupported) in [
            ("retry(code, budget)", "recursion"),
            ("budget / code > 0", "division"),
            ("let unused = 10 % code\n  true", "remainder"),
        ] {
            let program = [lower(
                &format!("fn retry(code :: Int, budget :: Int) -> Bool do\n  {body}\nend"),
                "Policy",
            )];
            let result = check(
                &program,
                function(&program[0], "Policy__retry"),
                function(&law, "decision"),
            );
            let Err(EncodingError { reason, .. }) = result else {
                panic!("{unsupported} received {result:?}");
            };
            assert!(reason.contains(unsupported), "{reason}");
        }
    }

    #[test]
    fn expected_expression_cannot_use_an_implementation_body() {
        let program = [lower(POLICY, "")];
        let law = lower(
            "fn eligible(status :: Int) -> Bool do false end\n\
             fn decision(status :: Int, left :: Int) -> Bool do eligible(status) end",
            "",
        );
        let result = check(
            &program,
            function(&program[0], "retry"),
            function(&law, "decision"),
        );
        let Err(EncodingError { reason, .. }) = result else {
            panic!("expected expression called an implementation function: {result:?}");
        };
        assert!(
            reason.contains("expected expression") && reason.contains("eligible"),
            "{reason}"
        );
    }

    #[test]
    fn decodes_full_signed_witness_and_checks_all_values() {
        let program = [lower(POLICY, "Policy")];
        let law = lower(LAW, "");
        let query = query(&program, &law);
        assert_eq!(
            parse_witness(
                &query,
                "((actual true) (expected false) (input0 #xffffffffffffffff) (input1 #x8000000000000000))",
            ),
            Ok(Verdict::Falsified {
                inputs: BTreeMap::from([("left".into(), Json::from(i64::MIN)), ("status".into(), Json::from(-1))]),
                expected: Json::from(false),
                actual: Json::from(true),
            }),
        );
        for invalid in [
            "((actual true) (expected true) (input0 #x0000000000000001) (input1 #x0000000000000000))",
            "((actual true) (expected false) (input0 #x0000000000000001) (input0 #x0000000000000000))",
            "((actual true) (expected false) (input0 #x1) (input1 #x0000000000000000))",
            "((actual true) (expected false) (input0 #x0000000000000001))",
            "((actual true) (expected false) (input0 #x0000000000000001) (input1 #x0000000000000000)))",
        ] {
            assert!(parse_witness(&query, invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn missing_solver_and_exhausted_budget_prove_nothing() {
        let program = [lower(POLICY, "Policy")];
        let law = lower(LAW, "");
        let query = query(&program, &law);
        let directory = tempfile::tempdir().unwrap();
        let absent = directory.path().join("missing-z3");
        let missing = Solver::new(
            absent.clone().into_os_string(),
            verify_process::DEFAULT_TIMEOUT_MS,
        )
        .solve(&query)
        .unwrap_err();
        assert!(missing.contains("could not start Z3"), "{missing}");
        assert_eq!(
            Solver::new(absent.into_os_string(), 0).solve(&query),
            Err("solver time budget exhausted".into()),
        );
    }

    #[cfg(unix)]
    #[test]
    fn kills_and_reaps_a_solver_that_exceeds_its_deadline() {
        let solver = Solver::new("/bin/sleep".into(), 30);
        assert_eq!(
            solver.run(&["10"], ""),
            Err("solver time budget exhausted".into())
        );
    }

    #[test]
    #[ignore = "requires optional pinned Z3; set MESH_VERIFY_Z3 or put it on PATH"]
    fn pinned_z3_verifies_and_finds_a_witness_beyond_the_finite_sweep() {
        let law = lower(LAW, "");
        let program = [lower(POLICY, "Policy")];
        assert_eq!(
            check(
                &program,
                function(&program[0], "Policy__retry"),
                function(&law, "decision")
            ),
            Ok(Verdict::Verified),
        );

        let capped = POLICY.replace("budget > 0", "budget > 0 && budget <= 8");
        let program = [lower(&capped, "Policy")];
        let verdict = check(
            &program,
            function(&program[0], "Policy__retry"),
            function(&law, "decision"),
        );
        let Ok(Verdict::Falsified {
            inputs,
            expected,
            actual,
        }) = verdict
        else {
            panic!("capped policy received {verdict:?}");
        };
        let status = inputs["status"].as_i64().unwrap();
        let budget = inputs["left"].as_i64().unwrap();
        assert!(status == 429 || status == 503, "{inputs:?}");
        assert!(budget > 8, "{inputs:?}");
        assert_eq!(expected, Json::from(true));
        assert_eq!(actual, Json::from(false));

        let mut query = query(&program, &law);
        query.assertions.insert_str(0, "(set-option :rlimit 1)\n");
        let reason = Solver::configured().unwrap().solve(&query).unwrap_err();
        assert!(
            reason.contains("Z3 returned unknown") && reason.contains("resource limit"),
            "{reason}"
        );
    }

    #[test]
    #[ignore = "requires optional pinned Z3; set MESH_VERIFY_Z3 or put it on PATH"]
    fn pinned_z3_uses_wrapping_int_and_boolean_semantics() {
        let program = [lower(
            r#"
fn retry(x :: Int, enabled :: Bool) -> Bool do
  let next = x + 1
  enabled && next < x
end
"#,
            "Policy",
        )];
        let law = lower(
            r#"
fn decision(num :: Int, on :: Bool) -> Bool do
  if on do
    num == 9223372036854775807
  else
    false
  end
end
"#,
            "",
        );
        assert_eq!(
            check(
                &program,
                function(&program[0], "Policy__retry"),
                function(&law, "decision")
            ),
            Ok(Verdict::Verified),
        );
    }

    #[test]
    #[ignore = "requires optional pinned Z3; set MESH_VERIFY_Z3 or put it on PATH"]
    fn pinned_z3_checks_the_typed_imported_helper_body() {
        let helper = mesh_parser::parse("pub fn positive(left :: Int) -> Bool do left > 0 end");
        let helper_typed = mesh_typeck::check(&helper);
        assert!(helper_typed.errors.is_empty(), "{:?}", helper_typed.errors);
        let exports = mesh_typeck::collect_exports(&helper, &helper_typed);
        let mut imports = mesh_typeck::ImportContext::empty();
        imports.module_exports.insert(
            "Budget".into(),
            mesh_typeck::ModuleExports::new("Budget".into(), &exports),
        );

        let policy = mesh_parser::parse(&format!(
            "from Budget import positive\n{}",
            POLICY.replace("budget > 0", "positive(budget)"),
        ));
        let policy_typed = mesh_typeck::check_with_imports(&policy, &imports);
        assert!(policy_typed.errors.is_empty(), "{:?}", policy_typed.errors);
        let program = [
            mesh_codegen::lower_module_to_mir_raw(
                &policy,
                &policy_typed,
                "Policy",
                &Default::default(),
                &Default::default(),
                &[],
            )
            .unwrap(),
            mesh_codegen::lower_module_to_mir_raw(
                &helper,
                &helper_typed,
                "Budget",
                &exports.functions.keys().cloned().collect(),
                &Default::default(),
                &[],
            )
            .unwrap(),
        ];
        let law = lower(LAW, "");
        assert_eq!(
            check(
                &program,
                function(&program[0], "Policy__retry"),
                function(&law, "decision")
            ),
            Ok(Verdict::Verified),
        );
    }
}
