use std::{fmt::Display, ops::Range};

use chumsky::prelude::*;
use error::Error;
use heck::ToUpperCamelCase;

/// Th    if is data structure holds a value and the context in which it was parsed.
/// This context is used for error reporting.
#[derive(Debug, Clone)]
pub struct Parsed<T> {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) value: T,
}

impl<T: std::hash::Hash> std::hash::Hash for Parsed<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.value.hash(state);
    }
}

impl<T: PartialEq> PartialEq<Self> for Parsed<T> {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

impl<T: Display> Display for Parsed<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.value.fmt(f)
    }
}

impl<T: Eq> Eq for Parsed<T> {}

impl<T: PartialOrd + PartialEq> PartialOrd<Self> for Parsed<T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.value.partial_cmp(&other.value)
    }
}

impl<T: Ord> Ord for Parsed<T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.value.cmp(&other.value)
    }
}

impl<T> Parsed<T> {
    pub(crate) fn map<U>(&self, f: impl Fn(&T) -> U) -> Parsed<U> {
        Parsed {
            value: f(&self.value),
            start: self.start,
            end: self.end,
        }
    }
}

fn ident() -> impl Parser<char, Parsed<String>, Error = Simple<char>> {
    filter(|c: &char| c.is_ascii_alphabetic())
        .chain(filter(|c: &char| c.is_ascii_alphanumeric() || *c == '_').repeated())
        .collect()
        .map_with_span(|value: String, span: Range<usize>| Parsed {
            start: span.start(),
            end: span.end(),
            value,
        })
}

fn ln() -> impl Parser<char, (), Error = Simple<char>> {
    just("\n").or(just("\n\r")).ignored()
}

fn space() -> impl Parser<char, (), Error = Simple<char>> {
    filter(|c: &char| c.is_whitespace() && *c != '\n')
        .repeated()
        .ignored()
}

fn blank() -> impl Parser<char, (), Error = Simple<char>> {
    // We want to escape valid SQL comment beginning with -- while not escaping our syntax --: or --!
    let comment = just("--")
        .then(none_of(":!").rewind())
        .then(none_of('\n').repeated());
    filter(|c: &char| c.is_whitespace())
        .ignored()
        .or(comment.ignored())
        .repeated()
        .ignored()
}

#[derive(Debug, Clone)]
pub struct NullableIdent {
    pub name: Parsed<String>,
    pub nullable: bool,
    pub inner_nullable: bool,
}

fn parse_nullable_ident() -> impl Parser<char, Vec<NullableIdent>, Error = Simple<char>> {
    space()
        .ignore_then(ident())
        .then(just('?').or_not())
        .then(just("[?]").or_not())
        .map(|((name, null), inner_null)| NullableIdent {
            name,
            nullable: null.is_some(),
            inner_nullable: inner_null.is_some(),
        })
        .then_ignore(space())
        .separated_by(just(','))
        .allow_trailing()
        .delimited_by(just('('), just(')'))
}

#[derive(Debug, Clone)]
pub struct TypeAnnotation {
    pub name: Parsed<String>,
    pub fields: Vec<NullableIdent>,
}

impl TypeAnnotation {
    fn parser() -> impl Parser<char, Self, Error = Simple<char>> {
        just("--:")
            .ignore_then(space())
            .ignore_then(ident())
            .then_ignore(space())
            .then(parse_nullable_ident().or_not())
            .map(|(name, fields)| Self {
                name,
                fields: fields.unwrap_or_default(),
            })
    }
}

#[derive(Debug)]
pub(crate) struct QuerySql {
    pub(crate) sql_str: String,
    pub(crate) bind_params: Vec<Parsed<String>>,
}

impl QuerySql {
    /// Escape sql string and pattern that are not bind
    fn sql_escaping() -> impl Parser<char, (), Error = Simple<char>> {
        // https://www.postgresql.org/docs/current/sql-syntax-lexical.html

        // ":bind" TODO is this possible ?
        let constant = none_of("\"")
            .repeated()
            .delimited_by(just("\""), just("\""))
            .ignored();
        // ':bind'
        let string = none_of("'")
            .repeated()
            .delimited_by(just("'"), just("'"))
            .ignored();
        // E'\':bind\''
        let c_style_string = just("\\'")
            .or(just("''"))
            .ignored()
            .or(none_of("'").ignored())
            .repeated()
            .delimited_by(just("e'").or(just("E'")), just("'"))
            .ignored();
        // $:bind$:bind$:bind$
        let dollar_tag = just("$").then(none_of("$").repeated()).then(just("$"));
        let dollar_quoted = none_of("$")
            .repeated()
            .delimited_by(dollar_tag.clone(), dollar_tag)
            .ignored();

        c_style_string
            .or(string)
            .or(constant)
            .or(dollar_quoted)
            // Non c_style_string e
            .or(one_of("eE").then(none_of("'").rewind()).ignored())
            // Non binding sql
            .or(none_of("\"':$eE").ignored())
            .repeated()
            .at_least(1)
            .ignored()
    }

    /// Parse all bind from an SQL query
    fn parse_bind() -> impl Parser<char, Vec<Parsed<String>>, Error = Simple<char>> {
        just(':')
            .ignore_then(ident())
            .separated_by(Self::sql_escaping())
            .allow_leading()
            .allow_trailing()
    }

    fn parser() -> impl Parser<char, Self, Error = Simple<char>> {
        none_of(";")
            .repeated()
            .then_ignore(just(';'))
            .collect::<String>()
            .map_with_span(|mut sql_str, span: Range<usize>| {
                let sql_start = span.start;
                let bind_params: Vec<_> = Self::parse_bind()
                    .parse(sql_str.clone())
                    .unwrap()
                    .into_iter()
                    .map(|mut it| {
                        it.start += sql_start;
                        it.end += sql_start;
                        it
                    })
                    .collect();

                // Normalize
                let mut deduped_bind_params = bind_params.clone();
                deduped_bind_params.sort_unstable();
                deduped_bind_params.dedup();

                for bind_param in bind_params.iter().rev() {
                    let index = deduped_bind_params
                        .iter()
                        .position(|bp| bp == bind_param)
                        .unwrap();
                    let start = bind_param.start - sql_start - 1;
                    let end = bind_param.end - sql_start - 1;
                    sql_str.replace_range(start..=end, &format!("${}", index + 1))
                }

                Self {
                    sql_str,
                    bind_params,
                }
            })
    }
}

#[derive(Debug)]
pub(crate) struct Query {
    pub(crate) annotation: QueryAnnotation,
    pub(crate) sql: QuerySql,
}

impl Query {
    fn parser() -> impl Parser<char, Self, Error = Simple<char>> {
        QueryAnnotation::parser()
            .then_ignore(space())
            .then_ignore(ln())
            .then(QuerySql::parser())
            .map(|(annotation, sql)| Self { annotation, sql })
    }
}

#[derive(Debug)]
pub(crate) enum QueryDataStruct {
    Implicit { idents: Vec<NullableIdent> },
    Named(Parsed<String>),
}

impl QueryDataStruct {
    pub(crate) fn name_and_fields(
        self,
        registered_structs: &[TypeAnnotation],
        query_name: &Parsed<String>,
        name_suffix: Option<&str>,
    ) -> (Vec<NullableIdent>, Parsed<String>) {
        match self {
            QueryDataStruct::Implicit { idents } => (
                idents,
                query_name.map(|x| {
                    format!(
                        "{}{}",
                        x.to_upper_camel_case(),
                        name_suffix.unwrap_or_default()
                    )
                }),
            ),
            QueryDataStruct::Named(name) => (
                registered_structs
                    .iter()
                    .find_map(|it| (it.name == name).then(|| it.fields.clone()))
                    .unwrap_or_default(),
                name,
            ),
        }
    }
}

impl Default for QueryDataStruct {
    fn default() -> Self {
        Self::Implicit { idents: Vec::new() }
    }
}

impl QueryDataStruct {
    fn parser() -> impl Parser<char, Self, Error = Simple<char>> {
        parse_nullable_ident()
            .map(|idents| Self::Implicit { idents })
            .or(ident().map(Self::Named))
    }
}

#[derive(Debug)]
pub(crate) struct QueryAnnotation {
    pub(crate) name: Parsed<String>,
    pub(crate) param: QueryDataStruct,
    pub(crate) row: QueryDataStruct,
}

impl QueryAnnotation {
    fn parser() -> impl Parser<char, Self, Error = Simple<char>> {
        just("--!")
            .ignore_then(space())
            .ignore_then(ident())
            .then_ignore(space())
            .then(QueryDataStruct::parser().or_not())
            .then_ignore(space())
            .then(
                just(':')
                    .ignore_then(space())
                    .ignore_then(QueryDataStruct::parser())
                    .or_not(),
            )
            .map(|((name, param), row)| Self {
                name,
                param: param.unwrap_or_default(),
                row: row.unwrap_or_default(),
            })
    }
}

#[derive(Debug)]
enum Statement {
    Type(TypeAnnotation),
    Query(Query),
}

#[derive(Debug)]
pub(crate) struct ParsedModule {
    pub(crate) types: Vec<TypeAnnotation>,
    pub(crate) queries: Vec<Query>,
}

impl FromIterator<Statement> for ParsedModule {
    fn from_iter<T: IntoIterator<Item = Statement>>(iter: T) -> Self {
        let mut types = Vec::new();
        let mut queries = Vec::new();
        for item in iter {
            match item {
                Statement::Type(it) => types.push(it),
                Statement::Query(it) => queries.push(it),
            }
        }
        ParsedModule { types, queries }
    }
}

/// Parse queries in in the input string using the grammar file (`grammar.pest`).
pub(crate) fn parse_query_module(path: &str, input: &str) -> Result<ParsedModule, Error> {
    TypeAnnotation::parser()
        .map(Statement::Type)
        .or(Query::parser().map(Statement::Query))
        .separated_by(blank())
        .allow_leading()
        .allow_trailing()
        .then_ignore(end())
        .collect()
        .parse(input)
        .map_err(|e| Error {
            path: path.to_string(),
            err: e,
        })
}

pub(crate) mod error {

    use thiserror::Error as ThisError;

    #[derive(Debug, ThisError)]
    #[error("Error while parsing queries [path: \"{path}\"]:\n{err:?}.")]
    pub struct Error {
        pub path: String,
        pub err: Vec<chumsky::error::Simple<char>>,
    }
}
