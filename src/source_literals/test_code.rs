//! The spans of the nodes a `#[cfg(test)]` outer attribute marks.

use proc_macro2::{Delimiter, LineColumn, TokenStream, TokenTree};
use quote::ToTokens;
use syn::visit::{self, Visit};

/// Where each node marked by an outer `#[cfg(test)]` starts and ends, read off the
/// parsed tree so a brace or generic comma inside a node never ends it early.
#[derive(Default)]
pub struct TestCode(Vec<(LineColumn, LineColumn)>);

impl TestCode {
    pub fn contains(&self, at: LineColumn) -> bool {
        self.0.iter().any(|&(start, end)| start <= at && at < end)
    }

    fn mark_if_test(&mut self, node: &impl ToTokens) -> bool {
        let tokens: Vec<TokenTree> = node.to_token_stream().into_iter().collect();
        let marked = tokens
            .chunks(2)
            .map_while(|pair| match pair {
                [TokenTree::Punct(p), TokenTree::Group(attr)]
                    if p.as_char() == '#' && attr.delimiter() == Delimiter::Bracket =>
                {
                    Some(attr)
                }
                _ => None,
            })
            .any(|attr| is_cfg_test(&attr.stream()));
        if let (true, Some(first), Some(last)) = (marked, tokens.first(), tokens.last()) {
            self.0.push((first.span().start(), last.span().end()));
        }
        marked
    }
}

macro_rules! skip_marked {
    ($($method:ident: $node:ident,)*) => {
        impl<'ast> Visit<'ast> for TestCode {
            $(fn $method(&mut self, node: &'ast syn::$node) {
                if !self.mark_if_test(node) {
                    visit::$method(self, node);
                }
            })*
        }
    };
}

skip_marked! {
    visit_item: Item,
    visit_impl_item: ImplItem,
    visit_trait_item: TraitItem,
    visit_foreign_item: ForeignItem,
    visit_field: Field,
    visit_variant: Variant,
    visit_arm: Arm,
    visit_fn_arg: FnArg,
    visit_stmt: Stmt,
    visit_expr: Expr,
    visit_field_value: FieldValue,
    visit_field_pat: FieldPat,
    visit_generic_param: GenericParam,
}

pub fn is_cfg_test(attr: &TokenStream) -> bool {
    attr.to_string().replace(' ', "") == "cfg(test)"
}
