use data::*;

use syntax::parse::token::intern;
use syntax::codemap::Span;
use syntax::util::small_vector::SmallVector;
use syntax::abi;
use syntax::ast::*;
use syntax::ast_util::empty_generics;
use syntax::ext::base::{MacResult, ExtCtxt, DummyResult, MacEager};
use syntax::ext::build::AstBuilder;
use syntax::parse::token::{self, InternedString};
use syntax::ptr::*;
use syntax::owned_slice::OwnedSlice;

use uuid::Uuid;

pub fn expand_cpp_include<'a>(ec: &'a mut ExtCtxt,
                              mac_span: Span,
                              tts: &[TokenTree]) -> Box<MacResult + 'a> {
    if tts.len() == 0 {
        ec.span_err(mac_span,
                    "unexpected empty cpp_include!");
        return DummyResult::any(mac_span);
    }

    let span = Span {
        lo: tts.first().unwrap().get_span().lo,
        hi: tts.last().unwrap().get_span().hi,
        expn_id: mac_span.expn_id,
    };

    let inner = ec.parse_sess.span_diagnostic.cm.span_to_snippet(span).unwrap();

    let mut headers = CPP_HEADERS.lock().unwrap();
    *headers = format!("{}\n#include {}\n", *headers, inner);

    MacEager::items(SmallVector::zero())
}

pub fn expand_cpp_header<'a>(ec: &'a mut ExtCtxt,
                             mac_span: Span,
                             tts: &[TokenTree]) -> Box<MacResult + 'a> {
    if tts.len() == 0 {
        ec.span_err(mac_span,
                    "unexpected empty cpp_header!");
        return DummyResult::any(mac_span);
    }

    let span = Span {
        lo: tts.first().unwrap().get_span().lo,
        hi: tts.last().unwrap().get_span().hi,
        expn_id: mac_span.expn_id,
    };

    let inner = ec.parse_sess.span_diagnostic.cm.span_to_snippet(span).unwrap();

    let mut headers = CPP_HEADERS.lock().unwrap();
    *headers = format!("{}\n{}\n", *headers, inner);

    MacEager::items(SmallVector::zero())
}

pub fn expand_cpp<'a>(ec: &'a mut ExtCtxt,
                      mac_span: Span,
                      tts: &[TokenTree]) -> Box<MacResult + 'a> {
    let mut parser = ec.new_parser_from_tts(tts);
    let mut captured_idents = Vec::new();

    // Parse the identifier list
    match parser.parse_token_tree().ok() {
        Some(TtDelimited(span, ref del)) => {
            let mut parser = ec.new_parser_from_tts(&del.tts[..]);
            loop {
                if parser.check(&token::Eof) { break }

                let mutable = parser.parse_mutability().unwrap_or(MutImmutable);
                let ident = parser.parse_ident().unwrap();
                captured_idents.push((ident, mutable));

                if !parser.eat(&token::Comma).unwrap() {
                    break
                }
            }
            if !parser.check(&token::Eof) {
                ec.span_err(span,
                            "Unexpected token in captured identifier list");
                return DummyResult::expr(span);
            }
        }
        Some(ref tt) => {
            ec.span_err(tt.get_span(),
                        "First argument to cpp! must be a list of captured identifiers");
            return DummyResult::expr(tt.get_span());
        }
        None => {
            ec.span_err(mac_span, "Unexpected empty cpp! macro invocation");
            return DummyResult::expr(mac_span);
        }
    }
/*
    // Check if we are looking at an ->
    let ret_ty = if parser.eat(&token::RArrow).unwrap() {
        parser.parse_ty()
    } else {
        ec.ty(mac_span, TyTup(Vec::new()))
    };
*/
    // Read in the body
    let body_tt = parser.parse_token_tree().unwrap();
    parser.expect(&token::Eof).unwrap();

    // Extract the string body of the c++ code
    let body_str = match body_tt {
        TtDelimited(span, ref del) => {
            if del.open_token() != token::OpenDelim(token::Brace) {
                ec.span_err(span, "cpp! body must be surrounded by `{}`");
                return DummyResult::expr(span);
            }

            ec.parse_sess.span_diagnostic.cm.span_to_snippet(span).unwrap()
        }
        _ => {
            ec.span_err(mac_span, "cpp! body must be a block surrounded by `{}`");
            return DummyResult::expr(body_tt.get_span());
        }
    };


    // Generate the rust parameters and arguments
    let mut typarams: Vec<_> = captured_idents.iter().enumerate().map(|(i,&(ref id, mutable))| {
        ec.typaram(mac_span, Ident::new(intern(&format!("T{}",i))), OwnedSlice::empty(), None)
    }).collect();
    typarams.push(ec.typaram(mac_span, Ident::new(intern("TR")), OwnedSlice::empty(), None));

    let params: Vec<_> = captured_idents.iter().enumerate().map(|(i,&(ref id, mutable))| {
        let arg_ty = ec.ty_ident(mac_span, Ident::new(intern(&format!("T{}", i))));
        ec.arg(mac_span, id.clone(), arg_ty)
    }).collect();

    let args: Vec<_> = captured_idents.iter().map(|&(ref id, mutable)| {
        ec.expr_ident(mac_span, id.clone())
        /*
        let arg_ty = ec.ty_ptr(mac_span,
                               ec.ty_ident(mac_span,
                                           Ident::new(intern("u8"))),
                               mutable);

        let addr_of = if mutable == MutImmutable {
            ec.expr_addr_of(mac_span, ec.expr_ident(mac_span, id.clone()))
        } else {
            ec.expr_mut_addr_of(mac_span, ec.expr_ident(mac_span, id.clone()))
        };

        ec.expr_cast(mac_span,
                     ec.expr_cast(mac_span,
                                  addr_of,
                                  ec.ty_ptr(mac_span,
                                            ec.ty_infer(mac_span),
                                            mutable)),
                     arg_ty)
        */
    }).collect();

    let fn_ident = Ident::new(intern(
        &format!("rust_cpp_{}", Uuid::new_v4().to_simple_string())));

    let body: &'static str = unsafe {::std::mem::transmute(&body_str as &str)};
    let fn_attrs = vec![
                    ec.attribute(
                        mac_span,
                        ec.meta_list(
                            mac_span,
                            InternedString::new("cpp_wrapper"),
                            vec![
                                ec.meta_name_value(
                                    mac_span,
                                    InternedString::new("body"),
                                    LitStr(InternedString::new(body),
                                           CookedStr))
                                ]))
                    ];

    // extern "C" declaration of function
    let foreign_mod = ForeignMod {
        abi: abi::CGeneric,
        items: vec![P(ForeignItem {
            ident: fn_ident.clone(),
            attrs: fn_attrs,
            node: ForeignItemFn(ec.fn_decl(params, ec.ty_ident(mac_span,
                                                               Ident::new(intern("TR")))),
                                    Generics {
                                        lifetimes: Vec::new(),
                                        ty_params: OwnedSlice::from_vec(typarams),
                                        where_clause: WhereClause {
                                            id: DUMMY_NODE_ID,
                                            predicates: Vec::new(),
                                        }
                                    }),
            id: DUMMY_NODE_ID,
            span: mac_span,
            vis: Inherited,
        })]
    };

    let mut link_attributes = Vec::new();

    {
        let fndecls = CPP_FNDECLS.lock().unwrap();
        let target = CPP_TARGET.lock().unwrap();

        let cxxlib = if target.contains("msvc") {
            None
        } else if target.contains("darwin") {
            Some("c++")
        } else {
            Some("stdc++")
        };

        if fndecls.is_empty() {
            if let Some(lib) = cxxlib {
                link_attributes.push(
                    ec.attribute(
                        mac_span,
                        ec.meta_list(
                            mac_span,
                            InternedString::new("link"),
                            vec![
                                ec.meta_name_value(
                                    mac_span,
                                    InternedString::new("name"),
                                    LitStr(InternedString::new(lib),
                                           CookedStr))
                                    ])));
            }

            link_attributes.push(
                ec.attribute(
                    mac_span,
                    ec.meta_list(
                        mac_span,
                        InternedString::new("link"),
                        vec![
                            ec.meta_name_value(
                                mac_span,
                                InternedString::new("name"),
                                LitStr(InternedString::new("rust_cpp_tmp"),
                                       CookedStr)),
                            ec.meta_name_value(
                                mac_span,
                                InternedString::new("kind"),
                                LitStr(InternedString::new("static"),
                                       CookedStr))
                                ])));
        }
    };

    let exp = ec.expr_block(
        // Block
        ec.block(
            mac_span,
            // Extern "C" declarations for the c implemented functions
            vec![ec.stmt_item(
                mac_span,
                ec.item(mac_span,
                        fn_ident.clone(),
                        link_attributes,
                        ItemForeignMod(foreign_mod)))],
            Some(ec.expr_call_ident(
                mac_span,
                fn_ident.clone(),
                args))));

    let cpp_decl = CppFn {
        name: format!("{}", fn_ident.name.as_str()),
        arg_idents: captured_idents.iter().map(|&(ref id, mutable)| CppParam {
            mutable: mutable == MutMutable,
            name: format!("{}", id.name.as_str()),
            ty: None,
        }).collect(),
        ret_ty: None,
        body: body_str,
        span: mac_span,
    };

    // Add the generated function declaration to the CPP_FNDECLS global variable.
    let mut fndecls = CPP_FNDECLS.lock().unwrap();
    fndecls.insert(fn_ident.name.as_str().to_string(), cpp_decl);

    // Emit the rust code into the AST
    MacEager::expr(exp)
}
