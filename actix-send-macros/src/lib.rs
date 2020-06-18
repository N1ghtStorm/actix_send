extern crate proc_macro;

use std::borrow::{Borrow, BorrowMut};

use proc_macro::TokenStream;
use quote::quote;
use syn::{
    punctuated::Punctuated, spanned::Spanned, AngleBracketedGenericArguments, AttrStyle, Attribute,
    AttributeArgs, Field, Fields, FieldsNamed, FnArg, GenericArgument, GenericParam, Ident,
    ImplItem, Item, Lit, Meta, MetaNameValue, NestedMeta, Pat, PatIdent, PatType, Path,
    PathArguments, PathSegment, PredicateType, TraitBound, TraitBoundModifier, Type, TypeParam,
    TypeParamBound, TypePath, Visibility, WhereClause, WherePredicate,
};

#[proc_macro_attribute]
pub fn actor(_meta: TokenStream, input: TokenStream) -> TokenStream {
    let item = syn::parse(input).expect("failed to parse input");

    match item {
        Item::Struct(mut struct_item) => {
            // collect struct fields and construct args as well as the struct fields' indent for create function
            let mut args_ident = Vec::new();
            let args = struct_item
                .fields
                .iter()
                .filter_map(|f| {
                    let ident = f.ident.as_ref()?;

                    args_ident.push(ident.clone());

                    let ty = f.ty.clone();
                    Some(FnArg::Typed(PatType {
                        attrs: vec![],
                        pat: Box::new(Pat::Ident(PatIdent {
                            attrs: vec![],
                            by_ref: None,
                            mutability: None,
                            ident: ident.clone(),
                            subpat: None,
                        })),
                        colon_token: Default::default(),
                        ty: Box::new(ty),
                    }))
                })
                .collect::<Vec<FnArg>>();

            // iter through the struct and make sure we have a unique generic type ident.
            let mut m = String::from("MESSAGE");
            loop {
                let ident = struct_item.generics.params.iter().find(|p| match *p {
                    GenericParam::Type(ty) => ty.ident == m.as_str(),
                    _ => false,
                });
                if ident.is_some() {
                    m.push_str("S");
                } else {
                    break;
                }
            }

            // construct ident for MESSAGE
            let m_ident = syn::Ident::new(m.as_str(), struct_item.span());

            // transform the MESSAGE ident to type param;
            let m_param = TypeParam {
                attrs: vec![],
                ident: m_ident.clone(),
                colon_token: None,
                bounds: Default::default(),
                eq_token: None,
                default: None,
            };

            // push our type param to struct's params
            struct_item
                .generics
                .params
                .push(GenericParam::Type(m_param.clone()));

            // construct a where clause if we don't have any.
            let where_clause = struct_item
                .generics
                .where_clause
                .get_or_insert(WhereClause {
                    where_token: Default::default(),
                    predicates: Default::default(),
                });

            // transform the MESSAGE type ident to type;
            let mut path = Path {
                leading_colon: None,
                segments: Punctuated::new(),
            };
            path.segments.push(PathSegment {
                ident: m_ident,
                arguments: Default::default(),
            });
            let m_b_type = syn::Type::Path(TypePath { qself: None, path });
            let mut m_p_type = PredicateType {
                lifetimes: None,
                // we would use this type later in struct field.
                bounded_ty: m_b_type.clone(),
                colon_token: Default::default(),
                bounds: Default::default(),
            };
            let mut path = Path {
                leading_colon: None,
                segments: Punctuated::new(),
            };
            path.segments.push(PathSegment {
                ident: syn::Ident::new("Message", path.span()),
                arguments: Default::default(),
            });
            m_p_type.bounds.push(TypeParamBound::Trait(TraitBound {
                paren_token: None,
                modifier: TraitBoundModifier::None,
                lifetimes: None,
                path,
            }));

            // push our MESSAGE: Message to where_clause;
            where_clause
                .predicates
                .push(WherePredicate::Type(m_p_type.clone()));

            // iter through struct fields to make sure we have a unique field name for PhantomData<MESSAGE>
            let mut phantom_field = String::from("_message");
            loop {
                let field = struct_item.fields.iter().find(|f| {
                    f.ident
                        .as_ref()
                        .map(|i| i == phantom_field.as_str())
                        .unwrap_or(false)
                });

                if field.is_some() {
                    phantom_field.push_str("s");
                } else {
                    break;
                }
            }

            // make phantom data type params
            let mut path = Path {
                leading_colon: None,
                segments: Default::default(),
            };

            let mut bracket = AngleBracketedGenericArguments {
                colon2_token: None,
                lt_token: Default::default(),
                args: Default::default(),
                gt_token: Default::default(),
            };

            bracket.args.push(GenericArgument::Type(m_b_type));

            path.segments.push(PathSegment {
                ident: syn::Ident::new("std", struct_item.span()),
                arguments: PathArguments::None,
            });

            path.segments.push(PathSegment {
                ident: syn::Ident::new("marker", struct_item.span()),
                arguments: PathArguments::None,
            });

            path.segments.push(PathSegment {
                ident: syn::Ident::new("PhantomData", struct_item.span()),
                arguments: PathArguments::AngleBracketed(bracket),
            });

            let ty = Type::Path(TypePath { qself: None, path });

            // construct a new field;
            let message_ident = syn::Ident::new(phantom_field.as_str(), phantom_field.span());

            match struct_item.fields.borrow_mut() {
                Fields::Named(fields) => fields.named.push(Field {
                    attrs: vec![],
                    vis: Visibility::Inherited,
                    ident: Some(message_ident.clone()),
                    colon_token: None,
                    ty: ty.clone(),
                }),
                Fields::Unnamed(fields) => fields.unnamed.push(Field {
                    attrs: vec![],
                    vis: Visibility::Inherited,
                    ident: None,
                    colon_token: None,
                    ty: ty.clone(),
                }),
                _ => {}
            }

            // special treat if we have a struct without fields
            let fields = if struct_item.fields.is_empty() {
                let mut named = FieldsNamed {
                    brace_token: Default::default(),
                    named: Default::default(),
                };

                named.named.push(Field {
                    attrs: vec![],
                    vis: Visibility::Inherited,
                    ident: Some(message_ident.clone()),
                    colon_token: None,
                    ty,
                });

                Fields::Named(named)
            } else {
                struct_item.fields
            };

            let ident = struct_item.ident;
            let vis = struct_item.vis;
            let semi_token = struct_item.semi_token;

            let (impl_gen, impl_ty, impl_where) = struct_item.generics.split_for_impl();

            // impl Actor trait for struct;
            let expended = quote! {
                    #vis struct #ident #impl_gen
                    #impl_where
                    #fields
                    #semi_token

                    impl #impl_gen #ident #impl_gen
                    #impl_where
                    {
                        pub fn create(#( #args ),*) -> #ident #impl_gen {
                            #ident {
                                #( #args_ident ),*, #message_ident: std::marker::PhantomData
                            }
                        }
                    }

                    impl #impl_gen Actor<#m_param> for #ident #impl_ty
                    #impl_where
                    {
                    }
            };

            expended.into()
        }

        _ => {
            unreachable!("Actor must be a struct");
        }
    }
}

const PANIC: &'static str = "message(result = \"T\") must be presented in attributes.";

#[proc_macro_attribute]
pub fn message(meta: TokenStream, input: TokenStream) -> TokenStream {
    let args = syn::parse_macro_input!(meta as AttributeArgs);
    let item = syn::parse_macro_input!(input as Item);

    let arg = args.first().expect(PANIC);

    let result = match arg {
        NestedMeta::Meta(meta) => {
            let _seg = meta
                .path()
                .segments
                .iter()
                .find(|s| s.ident == "result")
                .expect(PANIC);

            match meta {
                Meta::NameValue(MetaNameValue {
                    lit: Lit::Str(lit_str),
                    ..
                }) => syn::parse_str::<syn::Type>(lit_str.value().as_str()).expect(PANIC),
                _ => panic!(PANIC),
            }
        }
        _ => panic!(PANIC),
    };

    match item {
        Item::Struct(struct_item) => {
            let ident = &struct_item.ident;
            let (impl_gen, impl_ty, impl_where) = struct_item.generics.split_for_impl();

            let expended = quote! {
                    #struct_item

                    impl #impl_gen Message for #ident #impl_ty
                    #impl_where
                    {
                        type Result = #result;
                    }
            };

            expended.into()
        }
        Item::Enum(enum_item) => {
            let ident = &enum_item.ident;
            let (impl_gen, impl_ty, impl_where) = enum_item.generics.split_for_impl();

            let expended = quote! {
                    #enum_item

                    impl #impl_gen Message for #ident #impl_ty
                    #impl_where
                    {
                        type Result = #result;
                    }
            };

            expended.into()
        }
        _ => unreachable!("Message must be a struct"),
    }
}

#[proc_macro_attribute]
pub fn handler(_meta: TokenStream, input: TokenStream) -> TokenStream {
    let item = syn::parse_macro_input!(input as Item);

    match item {
        Item::Impl(mut impl_item) => {
            // check if async_trait attribute is presented
            let mut segments = Punctuated::new();

            segments.push(PathSegment {
                ident: Ident::new("async_trait", impl_item.span()),
                arguments: Default::default(),
            });

            if !impl_item.attrs.contains(&Attribute {
                pound_token: Default::default(),
                style: AttrStyle::Outer,
                bracket_token: Default::default(),
                path: Path {
                    leading_colon: None,
                    segments,
                },
                tokens: Default::default(),
            }) {
                panic!("async_trait macro is not presented (or placed above actix-send::handler attribute)")
            }

            // extract message's TypePath
            let msg_path = impl_item
                .items
                .iter()
                .find_map(|i| match i {
                    ImplItem::Method(method) => method.sig.inputs.iter().find_map(|args| {
                        if let FnArg::Typed(PatType { ty, .. }) = args {
                            if let Type::Path(path) = ty.borrow() {
                                return Some(path.clone());
                            }
                        };
                        None
                    }),
                    _ => None,
                })
                .expect("Message Type is not presented in handle method");

            // add message's type to Handler trait
            let _ = impl_item
                .trait_
                .iter_mut()
                .map(|(_, path, _)| {
                    let path_seg = path
                        .segments
                        .first_mut()
                        .map(|path_seg| {
                            if path_seg.ident.to_string().as_str() != "Handler" {
                                panic!("Handler trait is not presented");
                            }
                            path_seg
                        })
                        .expect("Handler trait has not PathSegment");

                    let mut args = AngleBracketedGenericArguments {
                        colon2_token: None,
                        lt_token: Default::default(),
                        args: Default::default(),
                        gt_token: Default::default(),
                    };

                    args.args
                        .push(GenericArgument::Type(Type::Path(msg_path.clone())));

                    path_seg.arguments = PathArguments::AngleBracketed(args)
                })
                .collect::<()>();

            // add or push message's type to Actor struct's type params.
            let self_ty = impl_item.self_ty.borrow_mut();

            if let Type::Path(TypePath { path, .. }) = self_ty {
                let Path { segments, .. } = path;
                let args = segments
                    .first_mut()
                    .map(|seg| &mut seg.arguments)
                    .expect("PathSegment is missing for Actor struct");

                match args {
                    PathArguments::None => {
                        let mut bracket = AngleBracketedGenericArguments {
                            colon2_token: None,
                            lt_token: Default::default(),
                            args: Default::default(),
                            gt_token: Default::default(),
                        };

                        bracket
                            .args
                            .push(GenericArgument::Type(Type::Path(msg_path.clone())));

                        let args_new = PathArguments::AngleBracketed(bracket);

                        *args = args_new;
                    }
                    PathArguments::AngleBracketed(ref mut bracket) => bracket
                        .args
                        .push(GenericArgument::Type(Type::Path(msg_path))),
                    _ => panic!("ParenthesizedGenericArguments is not supported"),
                }
            }

            let expended = quote! { #impl_item };

            expended.into()
        }
        _ => unreachable!("Handler must be a impl for actix_send::Handler trait"),
    }
}
