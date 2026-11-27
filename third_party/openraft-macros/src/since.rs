use std::str::FromStr;

use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::quote;
use syn::parse::Parser;
use syn::spanned::Spanned;

use crate::utils;

pub struct Since {
    pub(crate) version: Option<String>,
    pub(crate) date: Option<String>,
}

impl Since {
    /// Build a `Since` struct from the given attribute arguments.
    pub(crate) fn new(args: TokenStream) -> Result<Since, syn::Error> {
        let mut since = Since {
            version: None,
            date: None,
        };

        type AttributeArgs = syn::punctuated::Punctuated<syn::Meta, syn::Token![,]>;

        let parsed_args = AttributeArgs::parse_terminated.parse(args.clone())?;

        for arg in parsed_args {
            match arg {
                syn::Meta::NameValue(namevalue) => {
                    let q = namevalue
                        .path
                        .get_ident()
                        .ok_or_else(|| syn::Error::new_spanned(&namevalue, "Must have specified ident"))?;

                    let ident = q.to_string().to_lowercase();

                    match ident.as_str() {
                        "version" => {
                            since.set_version(namevalue.value.clone(), Spanned::span(&namevalue.value))?;
                        }

                        "date" => {
                            since.set_date(namevalue.value.clone(), Spanned::span(&namevalue.value))?;
                        }

                        name => {
                            let msg = format!(
                                "Unknown attribute {} is specified; expected one of: `version`, `date`",
                                name,
                            );
                            return Err(syn::Error::new_spanned(namevalue, msg));
                        }
                    }
                }
                other => {
                    return Err(syn::Error::new_spanned(other, "Unknown attribute inside the macro"));
                }
            }
        }

        if since.version.is_none() {
            return Err(syn::Error::new_spanned(
                proc_macro2::TokenStream::from(args),
                "Missing `version` attribute",
            ));
        }

        Ok(since)
    }
    /// Append a `since` doc such as `Since: 1.0.0` to the bottom of the doc section.
    pub(crate) fn append_since_doc(self, item: TokenStream) -> Result<TokenStream, syn::Error> {
        let item = proc_macro2::TokenStream::from(item);

        // Present docs to skip, in order to append `since` at bottom of doc section.
        let mut present_docs = vec![];

        // Tokens left after present docs.
        let mut last_non_doc = vec![];

        let mut it = item.clone().into_iter();
        loop {
            let Some(curr) = it.next() else {
                break;
            };
            let Some(next) = it.next() else {
                last_non_doc.push(curr);
                break;
            };

            if utils::is_doc(&curr, &next) {
                present_docs.push(curr);
                present_docs.push(next);
            } else {
                last_non_doc.push(curr);
                last_non_doc.push(next);
                break;
            }
        }

        let since_docs_str = if present_docs.is_empty() {
            format!(r#"#[doc = " {}"]"#, self.to_doc_string())
        } else {
            // If there are already docs, insert a blank line.
            format!(r#"#[doc = ""] #[doc = " {}"]"#, self.to_doc_string())
        };
        let since_docs = proc_macro2::TokenStream::from_str(&since_docs_str).unwrap();

        let present_docs: proc_macro2::TokenStream = present_docs.into_iter().collect();
        let last_non_docs: proc_macro2::TokenStream = last_non_doc.into_iter().collect();

        // Other non doc tokens.
        let other: proc_macro2::TokenStream = it.collect();

        let tokens = quote! {
            #present_docs
            #since_docs
            #last_non_docs
            #other
        };
        Ok(tokens.into())
    }

    /// Build doc string: `Since: 1.0.0, Date(2021-01-01)` or  `Since: 1.0.0`
    fn to_doc_string(&self) -> String {
        let mut s = String::new();
        s.push_str("Since: ");

        if let Some(version) = &self.version {
            s.push_str(version.as_str());
        }
        if let Some(date) = &self.date {
            s.push_str(&format!(", Date({})", date));
        }
        s
    }

    pub(crate) fn set_version(&mut self, ver_lit: syn::Expr, span: Span) -> Result<(), syn::Error> {
        if self.version.is_some() {
            return Err(syn::Error::new(span, "`version` set multiple times."));
        }

        let ver = Self::parse_str(ver_lit, "version", span)?;

        semver::Version::parse(&ver)
            .map_err(|_e| syn::Error::new(span, format!("`version`(`{}`) is not valid semver.", ver)))?;

        self.version = Some(ver);

        Ok(())
    }

    pub(crate) fn set_date(&mut self, date_lit: syn::Expr, span: Span) -> Result<(), syn::Error> {
        if self.date.is_some() {
            return Err(syn::Error::new(span, "`date` set multiple times."));
        }

        let date_str = Self::parse_str(date_lit, "date", span)?;

        if !is_valid_date(&date_str) {
            return Err(syn::Error::new(
                span,
                format!(
                    "`date`(`{}`) is not valid date string. Expected format: yyyy-mm-dd",
                    date_str
                ),
            ));
        }

        self.date = Some(date_str);

        Ok(())
    }

    /// Extract string from `foo` or `"foo"`
    fn parse_str(expr: syn::Expr, field: &str, span: Span) -> Result<String, syn::Error> {
        let s = match expr {
            syn::Expr::Lit(s) => match s.lit {
                syn::Lit::Str(s) => s.value(),
                syn::Lit::Verbatim(s) => s.to_string(),
                _ => {
                    return Err(syn::Error::new(
                        span,
                        format!("Failed to parse value of `{}` as string.", field),
                    ))
                }
            },
            syn::Expr::Verbatim(s) => s.to_string(),
            _ => {
                return Err(syn::Error::new(
                    span,
                    format!("Failed to parse value of `{}` as string.", field),
                ))
            }
        };
        Ok(s)
    }
}

/// Whether `s` is an existing calendar date written as `yyyy-mm-dd`.
fn is_valid_date(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }

    let (Some(year), Some(month), Some(day)) =
        (parse_digits(&b[0..4]), parse_digits(&b[5..7]), parse_digits(&b[8..10]))
    else {
        return false;
    };

    let is_leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let days_in_month = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap => 29,
        2 => 28,
        _ => return false,
    };

    (1..=days_in_month).contains(&day)
}

/// Parse a string of ASCII digits as a decimal number; `None` if any byte is not a digit.
fn parse_digits(b: &[u8]) -> Option<u32> {
    b.iter().try_fold(0u32, |acc, c| {
        c.is_ascii_digit().then(|| acc * 10 + u32::from(c - b'0'))
    })
}

#[cfg(test)]
mod tests {
    use super::is_valid_date;

    #[test]
    fn test_is_valid_date() {
        assert!(is_valid_date("2021-01-01"));
        assert!(is_valid_date("2021-12-31"));
        assert!(is_valid_date("2024-02-29"));
        assert!(is_valid_date("2000-02-29"));

        assert!(!is_valid_date("2021-01--"));
        assert!(!is_valid_date("2021-1-01"));
        assert!(!is_valid_date("2021-01-1"));
        assert!(!is_valid_date("2021/01/01"));
        assert!(!is_valid_date("2021-00-01"));
        assert!(!is_valid_date("2021-13-01"));
        assert!(!is_valid_date("2021-01-00"));
        assert!(!is_valid_date("2021-04-31"));
        assert!(!is_valid_date("2023-02-29"));
        assert!(!is_valid_date("1900-02-29"));
        assert!(!is_valid_date("+021-01-01"));
        assert!(!is_valid_date("2021-01-01 "));
        assert!(!is_valid_date(""));
    }
}
