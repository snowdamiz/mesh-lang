use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{Display, Write};

use mesh_common::span::Span;
use mesh_parser::logic::{Arm, Binary, Binder, Constructor, Declaration, Pattern, Proof, Term};

const SIGNED_INT_ORDER: &str = "
local instance (priority := high) : LT (BitVec 64) where
  lt left right := _root_.BitVec.slt left right = true
local instance (priority := high) : LE (BitVec 64) where
  le left right := _root_.BitVec.sle left right = true
local instance (left right : BitVec 64) : Decidable (left < right) :=
  inferInstanceAs (Decidable (_root_.BitVec.slt left right = true))
local instance (left right : BitVec 64) : Decidable (left ≤ right) :=
  inferInstanceAs (Decidable (_root_.BitVec.sle left right = true))

";

pub(super) struct Rendered {
    pub source: String,
    pub theorem_symbols: Vec<String>,
}

#[derive(Default)]
pub(super) struct NativeImports {
    pub(super) names: BTreeMap<String, String>,
    pub(super) constructors: BTreeMap<(String, String), NativeConstructor>,
}

pub(super) struct NativeConstructor {
    pub(super) symbol: String,
    pub(super) term_alias: Option<String>,
}

pub(super) fn render(
    declarations: &[Declaration],
    compiled: &NativeImports,
) -> Result<Rendered, String> {
    let mut names = BTreeSet::new();
    let mut proofs = BTreeMap::new();
    let mut theorems = BTreeSet::new();
    for declaration in declarations {
        let (name, span) = match declaration {
            Declaration::Inductive { name, span, .. }
            | Declaration::Definition { name, span, .. } => (name, *span),
            Declaration::Theorem { name, span, .. } => {
                theorems.insert(name);
                (name, *span)
            }
            Declaration::Proof { name, term, span } => {
                if proofs.insert(name, (term, *span)).is_some() {
                    return Err(at(*span, format_args!("duplicate proof for `{name}`")));
                }
                continue;
            }
        };
        if !names.insert(name) {
            return Err(at(span, format_args!("duplicate declaration `{name}`")));
        }
        if compiled.names.contains_key(name) {
            return Err(at(
                span,
                format_args!("declaration `{name}` conflicts with an import"),
            ));
        }
    }
    for declaration in declarations {
        if let Declaration::Proof { name, span, .. } = declaration {
            if !theorems.contains(name) {
                return Err(at(
                    *span,
                    format_args!("proof has no theorem named `{name}`"),
                ));
            }
        }
    }

    let mut renderer = Renderer::default();
    for (name, symbol) in &compiled.names {
        let symbol = symbol.strip_prefix("_root_.").unwrap_or(symbol);
        if !symbol.split('.').all(is_lean_identifier) {
            return Err(format!("invalid checked symbol for import `{name}`"));
        }
        renderer.globals.insert(name, format!("_root_.{symbol}"));
    }
    for ((owner, variant), imported) in &compiled.constructors {
        let symbol = &imported.symbol;
        let symbol = symbol.strip_prefix("_root_.").unwrap_or(symbol);
        if !compiled.names.contains_key(owner) || !symbol.split('.').all(is_lean_identifier) {
            return Err(format!("invalid checked constructor `{owner}.{variant}`"));
        }
        let (family, tag) = symbol
            .rsplit_once('.')
            .ok_or_else(|| format!("missing checked family for constructor `{owner}.{variant}`"))?;
        let mut constructor = ConstructorSymbol::new(format!("_root_.{family}"), tag);
        if let Some(term) = &imported.term_alias {
            if !is_lean_identifier(term) {
                return Err(format!(
                    "invalid checked constructor term `{owner}.{variant}`"
                ));
            }
            constructor.term_alias = Some(format!("_root_.{term}"));
        }
        renderer.register_constructor(owner, variant, constructor);
    }

    let mut theorem_symbols = Vec::new();
    for (index, declaration) in declarations.iter().enumerate() {
        let symbol = format!("mesh_logic_decl_{index}");
        match declaration {
            Declaration::Inductive {
                name,
                params,
                kind,
                constructors,
                span,
            } => {
                renderer
                    .inductive(index, name, &symbol, params, kind.as_ref(), constructors)
                    .map_err(|error| at(*span, error))?;
            }
            Declaration::Definition {
                name,
                params,
                result,
                body,
                span,
            } => {
                renderer
                    .definition(name, &symbol, params, result, body)
                    .map_err(|error| at(*span, error))?;
            }
            Declaration::Theorem {
                name,
                params,
                proposition,
                span,
                ..
            } => {
                let (proof, proof_span) = proofs
                    .get(name)
                    .ok_or_else(|| at(*span, format_args!("missing proof for theorem `{name}`")))?;
                let mut scope = Scope::new();
                let binders = renderer
                    .binders(params, &mut scope)
                    .map_err(|error| at(*span, error))?;
                let goal = renderer
                    .term(proposition, &scope)
                    .map_err(|error| at(*span, error))?;
                renderer.line(0, format_args!("theorem {symbol}{binders} : {goal} := by"));
                renderer
                    .proof(proof, &scope, 2)
                    .map_err(|error| at(*proof_span, error))?;
                renderer.globals.insert(name, symbol.clone());
                theorem_symbols.push(symbol);
            }
            Declaration::Proof { .. } => {}
        }
    }

    if renderer.uses_ordering {
        // The checked model represents Mesh Int as a two's-complement bitvector.
        renderer.source.insert_str(0, SIGNED_INT_ORDER);
    }
    Ok(Rendered {
        source: renderer.source,
        theorem_symbols,
    })
}

fn at(span: Span, message: impl Display) -> String {
    format!("source bytes {}..{}: {message}", span.start, span.end)
}

fn is_lean_identifier(part: &str) -> bool {
    let mut bytes = part.bytes();
    matches!(bytes.next(), Some(b'a'..=b'z' | b'A'..=b'Z' | b'_'))
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'\''))
        && part != "_"
}

type Scope<'a> = BTreeMap<&'a str, String>;

#[derive(Clone)]
struct ConstructorSymbol {
    family: String,
    tag: String,
    term_alias: Option<String>,
}

impl ConstructorSymbol {
    fn new(family: impl Into<String>, tag: impl Into<String>) -> Self {
        Self {
            family: family.into(),
            tag: tag.into(),
            term_alias: None,
        }
    }

    fn term(&self) -> String {
        self.term_alias.clone().unwrap_or_else(|| self.pattern())
    }

    fn pattern(&self) -> String {
        format!("{}.{}", self.family, self.tag)
    }
}

#[derive(Default)]
struct Renderer<'a> {
    source: String,
    globals: Scope<'a>,
    constructors: BTreeMap<(&'a str, &'a str), ConstructorSymbol>,
    unqualified: BTreeMap<&'a str, Option<&'a str>>,
    next_local: usize,
    uses_ordering: bool,
}

impl<'a> Renderer<'a> {
    fn line(&mut self, indent: usize, value: impl Display) {
        writeln!(self.source, "{:indent$}{value}", "").unwrap();
    }

    fn fresh_local(&mut self) -> String {
        let local = format!("_mesh_logic_local_{}", self.next_local);
        self.next_local += 1;
        local
    }

    fn binders(&mut self, binders: &'a [Binder], scope: &mut Scope<'a>) -> Result<String, String> {
        let mut result = String::new();
        let mut names = BTreeSet::new();
        for binder in binders {
            if !names.insert(binder.name.as_str()) {
                return Err(format!("duplicate binding `{}`", binder.name));
            }
            let ty = self.term(&binder.ty, scope)?;
            let local = self.fresh_local();
            write!(result, " ({local} : {ty})").unwrap();
            scope.insert(&binder.name, local);
        }
        Ok(result)
    }

    fn inductive(
        &mut self,
        index: usize,
        name: &'a str,
        symbol: &str,
        params: &'a [Binder],
        kind: Option<&'a Term>,
        constructors: &'a [Constructor],
    ) -> Result<(), String> {
        let mut scope = Scope::new();
        let binders = self.binders(params, &mut scope)?;
        let mut target = symbol.to_owned();
        for binder in params {
            write!(target, " {}", scope[binder.name.as_str()]).unwrap();
        }
        let kind = match kind {
            Some(kind) => self.term(kind, &scope)?,
            None => "Type".to_owned(),
        };
        self.line(
            0,
            format_args!("inductive {symbol}{binders} : {kind} where"),
        );
        self.globals.insert(name, symbol.to_owned());
        for (ordinal, constructor) in constructors.iter().enumerate() {
            let key = (name, constructor.name.as_str());
            if self.constructors.contains_key(&key) {
                return Err(format!(
                    "duplicate constructor `{name}.{}`",
                    constructor.name
                ));
            }
            let mut scope = scope.clone();
            let fields = self.binders(&constructor.fields, &mut scope)?;
            let result = if let Some(result) = &constructor.result {
                let mut head = result;
                while let Term::Call(function, _) = head {
                    head = function;
                }
                if !matches!(head, Term::Name(parts) if parts.as_slice() == [name])
                    || scope.contains_key(name)
                {
                    return Err(format!(
                        "constructor `{name}.{}` must return `{name}`",
                        constructor.name
                    ));
                }
                self.term(result, &scope)?
            } else {
                target.clone()
            };
            let tag = format!("mesh_logic_ctor_{index}_{ordinal}");
            self.line(2, format_args!("| {tag}{fields} : {result}"));
            self.register_constructor(name, &constructor.name, ConstructorSymbol::new(symbol, tag));
        }
        Ok(())
    }

    fn register_constructor(&mut self, owner: &'a str, name: &'a str, symbol: ConstructorSymbol) {
        self.constructors.insert((owner, name), symbol);
        self.unqualified
            .entry(name)
            .and_modify(|owner| *owner = None)
            .or_insert(Some(owner));
    }

    fn definition(
        &mut self,
        name: &'a str,
        symbol: &str,
        params: &'a [Binder],
        result: &'a Term,
        body: &'a Term,
    ) -> Result<(), String> {
        let mut scope = Scope::new();
        let binders = self.binders(params, &mut scope)?;
        let result = self.term(result, &scope)?;
        self.globals.insert(name, symbol.to_owned());
        let body = self.term(body, &scope)?;
        self.line(0, format_args!("def {symbol}{binders} : {result} :="));
        self.line(2, body);
        Ok(())
    }

    fn constructor(&self, parts: &[String]) -> Result<Option<ConstructorSymbol>, String> {
        match parts {
            [name] => match self.unqualified.get(name.as_str()) {
                Some(Some(owner)) => {
                    let constructor = &self.constructors[&(*owner, name.as_str())];
                    return Ok(Some(constructor.clone()));
                }
                Some(None) => {
                    return Err(format!(
                        "ambiguous constructor `{name}`; use a qualified name"
                    ));
                }
                None => {}
            },
            [owner, name] => {
                if let Some(constructor) = self.constructors.get(&(owner.as_str(), name.as_str())) {
                    return Ok(Some(constructor.clone()));
                }
                if self.globals.contains_key(owner.as_str()) {
                    return Ok(None);
                }
            }
            _ => {}
        }
        Ok(builtin_constructor(&parts.join(".")))
    }

    fn resolve(&self, parts: &[String], scope: &Scope<'_>) -> Result<String, String> {
        if let [name] = parts {
            if let Some(local) = scope
                .get(name.as_str())
                .or_else(|| self.globals.get(name.as_str()))
            {
                return Ok(local.clone());
            }
        }
        let name = parts.join(".");
        if let Some(constructor) = self.constructor(parts)? {
            return Ok(constructor.term());
        }
        if let [owner, _, ..] = parts {
            if self.globals.contains_key(owner.as_str()) {
                return Err(format!("unknown logic name `{name}`"));
            }
        }
        builtin_value(&name)
            .map(str::to_owned)
            .ok_or_else(|| format!("unknown logic name `{name}`"))
    }

    fn term(&mut self, term: &'a Term, scope: &Scope<'a>) -> Result<String, String> {
        match term {
            Term::Name(parts) => self.resolve(parts, scope),
            Term::Nat(value) => Ok(value.to_string()),
            Term::Bool(value) => Ok(format!("_root_.Bool.{value}")),
            Term::Call(function, arguments) => {
                let mut function = self.term(function, scope)?;
                if arguments.is_empty() {
                    return Ok(function);
                }
                for argument in arguments {
                    write!(function, " {}", self.term(argument, scope)?).unwrap();
                }
                Ok(format!("({function})"))
            }
            Term::Binary(operator, left, right) => {
                let left = self.term(left, scope)?;
                let right = self.term(right, scope)?;
                self.uses_ordering |=
                    matches!(operator, Binary::Lt | Binary::Le | Binary::Gt | Binary::Ge);
                let op = match operator {
                    Binary::Eq => "=",
                    Binary::Ne => "≠",
                    Binary::And => "∧",
                    Binary::Or => "∨",
                    Binary::Implies | Binary::Arrow => "→",
                    Binary::Add => "+",
                    Binary::Sub => "-",
                    Binary::Mul => "*",
                    Binary::Lt => "<",
                    Binary::Le => "≤",
                    Binary::Gt => ">",
                    Binary::Ge => "≥",
                };
                Ok(format!("({left} {op} {right})"))
            }
            Term::Forall(binder, body)
            | Term::Exists(binder, body)
            | Term::Lambda(binder, body) => {
                let ty = self.term(&binder.ty, scope)?;
                let local = self.fresh_local();
                let mut inner = scope.clone();
                inner.insert(&binder.name, local.clone());
                let body = self.term(body, &inner)?;
                Ok(match term {
                    Term::Forall(..) => format!("(∀ ({local} : {ty}), {body})"),
                    Term::Exists(..) => format!("(∃ ({local} : {ty}), {body})"),
                    Term::Lambda(..) => format!("(fun ({local} : {ty}) => {body})"),
                    _ => unreachable!(),
                })
            }
            Term::Annotate(value, ty) => Ok(format!(
                "({} : {})",
                self.term(value, scope)?,
                self.term(ty, scope)?
            )),
            Term::Let(binder, value, body) => {
                let value = self.term(value, scope)?;
                let ty = self.term(&binder.ty, scope)?;
                let local = self.fresh_local();
                let mut inner = scope.clone();
                inner.insert(&binder.name, local.clone());
                let body = self.term(body, &inner)?;
                Ok(format!("(let {local} : {ty} := {value}; {body})"))
            }
            Term::Pair(left, right) => Ok(format!(
                "⟨{}, {}⟩",
                self.term(left, scope)?,
                self.term(right, scope)?
            )),
            Term::If(condition, yes, no) => Ok(format!(
                "(if {} then {} else {})",
                self.term(condition, scope)?,
                self.term(yes, scope)?,
                self.term(no, scope)?
            )),
            Term::Match(subject, arms) => {
                if arms.is_empty() {
                    return Err("match requires at least one arm".into());
                }
                let mut result = format!("(match {} with", self.term(subject, scope)?);
                for arm in arms {
                    let mut branch = scope.clone();
                    let pattern = self.pattern(&arm.pattern, &mut branch)?;
                    write!(result, " | {pattern} => {}", self.term(&arm.body, &branch)?).unwrap();
                }
                result.push(')');
                Ok(result)
            }
        }
    }

    fn pattern(&mut self, pattern: &'a Pattern, scope: &mut Scope<'a>) -> Result<String, String> {
        match pattern {
            Pattern::Constructor(parts, bindings) => {
                let mut constructor = self.require_constructor(parts)?.pattern();
                constructor.push_str(&self.pattern_bindings(bindings, scope)?);
                Ok(constructor)
            }
            Pattern::Wildcard => Ok("_".into()),
            Pattern::Bool(value) => Ok(format!("_root_.Bool.{value}")),
            Pattern::Nat(value) => Ok(value.to_string()),
        }
    }

    fn require_constructor(&self, parts: &[String]) -> Result<ConstructorSymbol, String> {
        self.constructor(parts)?
            .ok_or_else(|| format!("unknown logic constructor `{}`", parts.join(".")))
    }

    fn pattern_bindings(
        &mut self,
        bindings: &'a [String],
        scope: &mut Scope<'a>,
    ) -> Result<String, String> {
        let mut result = String::new();
        let mut names = BTreeSet::new();
        for name in bindings {
            if name == "_" {
                result.push_str(" _");
                continue;
            }
            if !names.insert(name) {
                return Err(format!("duplicate pattern binding `{name}`"));
            }
            let local = self.fresh_local();
            write!(result, " {local}").unwrap();
            scope.insert(name, local);
        }
        Ok(result)
    }

    fn proof(&mut self, proof: &'a Proof, scope: &Scope<'a>, indent: usize) -> Result<(), String> {
        match proof {
            Proof::Refl => self.line(indent, "rfl"),
            Proof::Goal => {
                self.line(indent, "trace_state");
                self.line(indent, "fail \"open Mesh proof goal\"");
            }
            Proof::Exact(term) => {
                let term = self.term(term, scope)?;
                self.line(indent, format_args!("exact {term}"));
            }
            Proof::Intro(name, rest) => {
                let local = self.fresh_local();
                self.line(indent, format_args!("intro {local}"));
                let mut inner = scope.clone();
                inner.insert(name, local);
                self.proof(rest, &inner, indent)?;
            }
            Proof::Split(left, right) => {
                self.line(indent, "refine _root_.And.intro ?_ ?_");
                self.line(indent, "·");
                self.proof(left, scope, indent + 2)?;
                self.line(indent, "·");
                self.proof(right, scope, indent + 2)?;
            }
            Proof::Witness(term, rest) => {
                let term = self.term(term, scope)?;
                self.line(indent, format_args!("refine _root_.Exists.intro {term} ?_"));
                self.proof(rest, scope, indent)?;
            }
            Proof::Cases(variable, arms) => {
                let (subject, remaining) = proof_subject(variable, scope)?;
                if arms.is_empty() {
                    self.line(indent, format_args!("exact nomatch {subject}"));
                    return Ok(());
                }
                self.line(indent, format_args!("match {subject} with"));
                for arm in arms {
                    let mut branch = remaining.clone();
                    let pattern = self.pattern(&arm.pattern, &mut branch)?;
                    self.line(indent, format_args!("| {pattern} =>"));
                    self.proof(&arm.body, &branch, indent + 2)?;
                }
            }
            Proof::Induction(variable, arms) => self.induction(variable, arms, scope, indent)?,
            Proof::Rewrite(term, rest) => {
                let term = self.term(term, scope)?;
                self.line(indent, format_args!("rewrite [{term}]"));
                self.proof(rest, scope, indent)?;
            }
            Proof::Branch(condition, yes_name, yes, no_name, no) => {
                let hypothesis = self.fresh_local();
                let condition = self.term(condition, scope)?;
                self.line(indent, format_args!("by_cases {hypothesis} : {condition}"));
                for (name, proof) in [(yes_name, yes), (no_name, no)] {
                    let mut branch = scope.clone();
                    branch.insert(name, hypothesis.clone());
                    self.line(indent, "·");
                    self.proof(proof, &branch, indent + 2)?;
                }
            }
            Proof::Change(goal, rest) => {
                let goal = self.term(goal, scope)?;
                self.line(indent, format_args!("change {goal}"));
                self.proof(rest, scope, indent)?;
            }
            Proof::Unfold(name, rest) => {
                let definition = self.resolve(name, scope)?;
                self.line(indent, format_args!("unfold {definition}"));
                self.proof(rest, scope, indent)?;
            }
            Proof::Left(rest) | Proof::Right(rest) => {
                let side = if matches!(proof, Proof::Left(_)) {
                    "inl"
                } else {
                    "inr"
                };
                self.line(indent, format_args!("refine _root_.Or.{side} ?_"));
                self.proof(rest, scope, indent)?;
            }
        }
        Ok(())
    }

    fn induction(
        &mut self,
        variable: &str,
        arms: &'a [Arm<Proof>],
        scope: &Scope<'a>,
        indent: usize,
    ) -> Result<(), String> {
        if arms.is_empty() {
            return Err("induction requires at least one arm".into());
        }
        let (subject, remaining) = proof_subject(variable, scope)?;
        let mut family = None;
        let mut branches = Vec::new();
        for arm in arms {
            let (constructor, bindings) = match &arm.pattern {
                Pattern::Constructor(parts, bindings) => (Some(self.require_constructor(parts)?), bindings.as_slice()),
                Pattern::Bool(value) => (Some(ConstructorSymbol::new("_root_.Bool", value.to_string())), &[][..]),
                Pattern::Nat(0) => (Some(ConstructorSymbol::new("_root_.Nat", "zero")), &[][..]),
                Pattern::Nat(_) => return Err("induction requires a constructor pattern; use `Nat.succ` for positive naturals".into()),
                Pattern::Wildcard => (None, &[][..]),
            };
            let tag = if let Some(constructor) = constructor {
                if family
                    .as_ref()
                    .is_some_and(|name| name != &constructor.family)
                {
                    return Err("induction arms must use constructors from the same type".into());
                }
                family.get_or_insert(constructor.family);
                constructor.tag
            } else {
                "_".into()
            };
            branches.push((tag, bindings));
        }
        match family {
            Some(family) => self.line(
                indent,
                format_args!("induction {subject} using {family}.rec with"),
            ),
            None => self.line(indent, format_args!("induction {subject} with")),
        }
        for (arm, (tag, bindings)) in arms.iter().zip(branches) {
            let mut branch = remaining.clone();
            let bindings = self.pattern_bindings(bindings, &mut branch)?;
            self.line(indent, format_args!("| {tag}{bindings} =>"));
            self.proof(&arm.body, &branch, indent + 2)?;
        }
        Ok(())
    }
}

fn proof_subject<'a, 'b>(
    variable: &str,
    scope: &'b Scope<'a>,
) -> Result<(&'b str, Scope<'a>), String> {
    let subject = scope
        .get(variable)
        .ok_or_else(|| format!("unknown proof binding `{variable}`"))?;
    let mut remaining = scope.clone();
    remaining.remove(variable);
    Ok((subject, remaining))
}

fn builtin_constructor(name: &str) -> Option<ConstructorSymbol> {
    let (family, tag) = match name {
        "List.nil" | "List.Nil" | "nil" | "Nil" => ("_root_.List", "nil"),
        "List.cons" | "List.Cons" | "cons" | "Cons" => ("_root_.List", "cons"),
        "Nat.zero" | "Nat.Zero" | "Zero" => ("_root_.Nat", "zero"),
        "Nat.succ" | "Nat.Succ" | "Succ" => ("_root_.Nat", "succ"),
        "Bool.false" => ("_root_.Bool", "false"),
        "Bool.true" => ("_root_.Bool", "true"),
        "And.intro" => ("_root_.And", "intro"),
        "Or.inl" => ("_root_.Or", "inl"),
        "Or.inr" => ("_root_.Or", "inr"),
        "Exists.intro" => ("_root_.Exists", "intro"),
        "Eq.refl" => ("_root_.Eq", "refl"),
        "True.intro" => ("_root_.True", "intro"),
        "Unit.unit" => ("_root_.Unit", "unit"),
        "Prod.mk" => ("_root_.Prod", "mk"),
        "Sigma.mk" => ("_root_.Sigma", "mk"),
        "Subtype.mk" => ("_root_.Subtype", "mk"),
        "List.Pairwise.nil" => ("_root_.List.Pairwise", "nil"),
        "List.Pairwise.cons" => ("_root_.List.Pairwise", "cons"),
        "List.Perm.cons" => ("_root_.List.Perm", "cons"),
        "List.Perm.swap" => ("_root_.List.Perm", "swap"),
        "List.Perm.trans" => ("_root_.List.Perm", "trans"),
        _ => return None,
    };
    Some(ConstructorSymbol::new(family, tag))
}

fn builtin_value(name: &str) -> Option<&'static str> {
    Some(match name {
        "Type" => "Type",
        "Prop" => "Prop",
        "Nat" => "_root_.Nat",
        "Int" => "(_root_.BitVec 64)",
        "Bool" => "_root_.Bool",
        "List" => "_root_.List",
        "Unit" => "_root_.Unit",
        "Empty" => "_root_.Empty",
        "Prod" => "_root_.Prod",
        "Sigma" => "_root_.Sigma",
        "Sigma.fst" => "_root_.Sigma.fst",
        "Sigma.snd" => "_root_.Sigma.snd",
        "Subtype" => "_root_.Subtype",
        "Subtype.val" => "_root_.Subtype.val",
        "Subtype.property" => "_root_.Subtype.property",
        "True" => "_root_.True",
        "False" => "_root_.False",
        "And" => "_root_.And",
        "Or" => "_root_.Or",
        "Eq" => "_root_.Eq",
        "Not" => "_root_.Not",
        "congr" => "_root_.congrArg",
        "Eq.symm" => "_root_.Eq.symm",
        "Eq.trans" => "_root_.Eq.trans",
        "And.left" => "_root_.And.left",
        "And.right" => "_root_.And.right",
        "False.elim" => "_root_.False.elim",
        "Empty.elim" => "_root_.Empty.elim",
        "List.append" => "_root_.List.append",
        "List.length" => "_root_.List.length",
        "List.Pairwise" => "_root_.List.Pairwise",
        "List.Pairwise.of_cons" => "_root_.List.Pairwise.of_cons",
        "List.rel_of_pairwise_cons" => "_root_.List.rel_of_pairwise_cons",
        "List.Perm" => "_root_.List.Perm",
        "List.Perm.refl" => "_root_.List.Perm.refl",
        "List.Perm.mem_iff" => "_root_.List.Perm.mem_iff",
        "List.Mem" => "_root_.List.Mem",
        "List.eq_or_mem_of_mem_cons" => "_root_.List.eq_or_mem_of_mem_cons",
        "Iff.mp" => "_root_.Iff.mp",
        "Iff.mpr" => "_root_.Iff.mpr",
        "mt" => "_root_.mt",
        "ite_eq_left" => "_root_.ite_eq_left",
        "ite_eq_right" => "_root_.ite_eq_right",
        "BitVec.sle" => "_root_.BitVec.sle",
        "BitVec.sle_iff_toInt_le" => "_root_.BitVec.sle_iff_toInt_le",
        "Int.le_trans" => "_root_.Int.le_trans",
        "Int.le_of_not_le" => "_root_.Int.le_of_not_le",
        "Prod.fst" => "_root_.Prod.fst",
        "Prod.snd" => "_root_.Prod.snd",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verify_proof::{check_artifact, Settings, PRELUDE};
    use mesh_parser::logic::parse;

    #[test]
    fn renders_and_checks_recursive_and_dependent_logic() {
        let program = parse(
            r#"logic
inductive Tree(T :: Type) do
  Leaf(value :: T)
  Branch(left :: Tree<T>, right :: Tree<T>)
end
fn copy(T :: Type, tree :: Tree<T>) -> Tree<T> do
  match tree do
    Leaf(value) -> Tree.Leaf(value)
    Branch(a, b) -> Tree.Branch(copy(T, a), copy(T, b))
  end
end
theorem copy_id(T :: Type, tree :: Tree<T>) do copy(T, tree) == tree end
proof copy_id do
  induction tree do
    Leaf(value) -> refl
    Tree.Branch(a, b, ih_a, ih_b) -> exact Eq.trans(
      congr(fn x :: Tree<T> -> Tree.Branch(x, copy(T, b)) end, ih_a),
      congr(Tree.Branch(a), ih_b),
    )
  end
end
theorem copied_leaf(value :: Nat) do copy(Nat, Tree.Leaf(value)) == Tree.Leaf(value) end
proof copied_leaf do exact copy_id(Nat, Tree.Leaf(value)) end
theorem dependent(n :: Nat, h :: n == 0) do n == 0 end
proof dependent do cases n do
  0 -> exact h
  Nat.succ(k) -> exact h
end end
theorem quantified(x :: Nat) do
  forall y :: Nat do x == y => (exists z :: Nat do z == x end) && (x != y || x == y) end
end
proof quantified do
  intro y do intro equal do
    split do
      witness y do rewrite equal do refl end end,
      right do exact equal end
    end
  end end
end
theorem signed_order() do
  external(18446744073709551615) < 0 &&
    (if external(18446744073709551615) <= 0 do true else false end) == true
end
proof signed_order do split do refl, refl end end
"#,
        )
        .unwrap();
        let compiled = NativeImports {
            names: BTreeMap::from([("external".into(), "mesh_import_0".into())]),
            ..Default::default()
        };
        let rendered =
            render(&program.declarations, &compiled).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            rendered.theorem_symbols,
            [
                "mesh_logic_decl_2",
                "mesh_logic_decl_4",
                "mesh_logic_decl_6",
                "mesh_logic_decl_8",
                "mesh_logic_decl_10",
            ]
        );
        assert!(!rendered.source.contains("#print"));
        if std::env::var_os("MESH_VERIFY_LEAN").is_some() {
            let source = format!(
                "{PRELUDE}def mesh_import_0 (a : BitVec 64) := a\n{}",
                rendered.source
            );
            check_artifact(
                source,
                &rendered.theorem_symbols,
                Settings {
                    emit: None,
                    verdict: false,
                },
            )
            .unwrap_or_else(|error| panic!("{error}"));
        }
    }

    #[test]
    fn checks_indexed_constructors_induction_and_empty_elimination() {
        let program = r#"logic
inductive Vec(T :: Type) -> Nat -> Type do
  End -> Vec<T, 0>
  Push(n :: Nat, head :: T, tail :: Vec<T, n>) -> Vec<T, Nat.succ(n)>
end

fn vec_head(T :: Type, n :: Nat, values :: Vec<T, n>, fallback :: T) -> T do
  match values do
    Vec.End -> fallback
    Vec.Push(_, head, _) -> head
  end
end

theorem head_of_push(T :: Type, head :: T, fallback :: T) do
  vec_head(T, 1, Vec.Push(0, head, Vec.End()), fallback) == head
end
proof head_of_push do refl end

theorem vec_induction(T :: Type, n :: Nat, values :: Vec<T, n>) do values == values end
proof vec_induction do
  induction values do
    Vec.End -> refl
    Vec.Push(k, head, tail, ih) -> exact congr(Vec.Push(k, head), ih)
  end
end

inductive Positive -> Nat -> Prop do
  One -> Positive<1>
  Step(n :: Nat, prev :: Positive<n>) -> Positive<Nat.succ(n)>
end

theorem positive_induction(n :: Nat, h :: Positive<n>) do True end
proof positive_induction do
  induction h do
    Positive.One -> exact True.intro
    Positive.Step(k, prev, ih) -> exact ih
  end
end

theorem zero_is_impossible(h :: Positive<0>) do False end
proof zero_is_impossible do cases h do end end

theorem empty_cases(h :: Empty) do False end
proof empty_cases do cases h do end end

theorem empty_elim(h :: Empty) do False end
proof empty_elim do exact Empty.elim(h) end
"#;
        let render_program = |source| {
            let file = parse(source).unwrap();
            render(&file.declarations, &NativeImports::default())
        };
        let rendered = render_program(program).unwrap();
        let wrong_family = program.replace("End -> Vec<T, 0>", "End -> Nat");
        let error = render_program(&wrong_family).err().unwrap();
        assert!(error.starts_with("source bytes "), "{error}");
        assert!(
            error.ends_with("constructor `Vec.End` must return `Vec`"),
            "{error}"
        );

        if std::env::var_os("MESH_VERIFY_LEAN").is_some() {
            let check = |rendered: Rendered| {
                check_artifact(
                    format!("{PRELUDE}{}", rendered.source),
                    &rendered.theorem_symbols,
                    Settings {
                        emit: None,
                        verdict: false,
                    },
                )
            };
            check(rendered).unwrap_or_else(|error| panic!("{error}"));
            let inhabited =
                render_program(&program.replace("h :: Positive<0>", "h :: Positive<1>")).unwrap();
            let error = check(inhabited).unwrap_err();
            assert!(error.contains("Missing cases:"), "{error}");
        }
    }

    #[test]
    fn rejects_invalid_names_and_proof_associations_with_source_ranges() {
        for (declarations, message) in [
            ("theorem t() do True end", "missing proof for theorem `t`"),
            ("proof t do refl end", "proof has no theorem named `t`"),
            (
                "theorem t() do True end\nproof t do refl end\nproof t do refl end",
                "duplicate proof for `t`",
            ),
            (
                "theorem t() do True end\nfn t() -> Nat do 0 end",
                "duplicate declaration `t`",
            ),
            (
                "theorem t() do False end\nproof t do exact Lean.sorryAx(False) end",
                "unknown logic name `Lean.sorryAx`",
            ),
            (
                concat!(
                    "theorem t() do True end\nproof t do exact later() end\n",
                    "theorem later() do True end\nproof later do exact True.intro end",
                ),
                "unknown logic name `later`",
            ),
        ] {
            let file = parse(&format!("logic\n{declarations}")).unwrap();
            let error = render(&file.declarations, &NativeImports::default())
                .err()
                .expect("invalid declarations rendered");
            assert!(error.starts_with("source bytes "), "{error}");
            assert!(error.ends_with(message), "{error}");
        }
        let compiled = NativeImports {
            names: BTreeMap::from([("unsafe".into(), "helper\naxiom injected".into())]),
            ..Default::default()
        };
        assert!(render(&[], &compiled).is_err());
    }
}
