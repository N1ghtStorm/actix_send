/// A set of macros for transform and generate code for actix_send crate
extern crate proc_macro;

use proc_macro::TokenStream;

use syn::{
    export::Span, punctuated::Punctuated, token::Paren, AngleBracketedGenericArguments, Arm,
    AttrStyle, Attribute, AttributeArgs, Block, Expr, ExprAsync, ExprAwait, ExprBlock, ExprCall,
    ExprClosure, ExprMacro, ExprMatch, ExprPath, Field, Fields, FieldsUnnamed, FnArg,
    GenericArgument, Generics, Ident, ImplItem, ImplItemMethod, ImplItemType, Item, ItemEnum,
    ItemImpl, Lit, Local, Macro, MacroDelimiter, Meta, MetaNameValue, NestedMeta,
    ParenthesizedGenericArguments, Pat, PatIdent, PatTuple, PatTupleStruct, PatType, PatWild, Path,
    PathArguments, PathSegment, Receiver, ReturnType, Signature, Stmt, Type, TypePath, Variant,
    VisPublic, Visibility,
};

use crate::message::{ActorInfo, HandleMethodInfo};
use quote::quote;

mod message;

#[proc_macro_attribute]
pub fn actor(meta: TokenStream, input: TokenStream) -> TokenStream {
    let item = syn::parse(input).expect("failed to parse input");

    let args = syn::parse_macro_input!(meta as AttributeArgs);

    match item {
        Item::Struct(struct_item) => {
            // check if #[actor(no_static)] attr is presented
            let is_dynamic = args
                .iter()
                .filter_map(|nest| {
                    if let NestedMeta::Meta(Meta::Path(Path { segments, .. })) = nest {
                        return Some(segments);
                    }

                    None
                })
                .map(|segments| {
                    segments
                        .iter()
                        .any(|seg| seg.ident.to_string().as_str() == "no_static")
                })
                .find(|bool| *bool)
                .unwrap_or(false);

            let actor_info = ActorInfo::new(&struct_item.ident);

            let actor_ident = actor_info.ident;
            let message_enum_ident = actor_info.message_enum_ident;
            let result_enum_ident = actor_info.result_enum_ident;

            let (impl_gen, impl_ty, impl_where) = struct_item.generics.split_for_impl();

            let expand = if is_dynamic {
                let attr = attr_from_ident_str(vec!["actix_send", "prelude", "async_trait"]);

                // impl dummy Message/Result type and dummy handler trait if we are a dynamic only
                // actor
                quote! {
                    #struct_item

                    impl #impl_gen Actor for #actor_ident #impl_ty
                    #impl_where
                    {
                        type Message = ();
                        type Result = ();
                        // ToDo: add on_start on_stop
                    }

                    #attr
                    impl Handler for #actor_ident
                    {
                        async fn handle(&mut self, msg: ()) {

                        }
                    }
                }
            } else {
                quote! {
                    #struct_item

                    impl #impl_gen Actor for #actor_ident #impl_ty
                    #impl_where
                    {
                        type Message = #message_enum_ident;
                        type Result = #result_enum_ident;

                        fn on_start(&mut self) {
                            self.__on_start();
                        }

                        fn on_stop(&mut self) {
                           self.__on_stop();
                        }
                    }
                }
            };

            expand.into()
        }
        _ => {
            unreachable!("Actor must be a struct");
        }
    }
}

const PANIC: &str = "message(result = \"T\") must be presented in attributes.";

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

    static_message(item, result)
}

#[proc_macro_attribute]
pub fn handler(_meta: TokenStream, input: TokenStream) -> TokenStream {
    let item = syn::parse_macro_input!(input as Item);

    match item {
        Item::Impl(mut impl_item) => {
            // add async_trait attribute if not presented.
            let async_trait_attr = attr_from_ident_str(vec!["async_trait"]);

            if !impl_item.attrs.contains(&async_trait_attr) {
                impl_item.attrs.push(async_trait_attr);
            }

            let expended = quote! { #impl_item };

            expended.into()
        }
        _ => unreachable!("Handler must be a impl for actix_send::Handler trait"),
    }
}

// Take a mod contains actor/messages/actor and pack all the messages into a actor.
#[proc_macro_attribute]
pub fn actor_mod(_meta: TokenStream, input: TokenStream) -> TokenStream {
    let item = syn::parse_macro_input!(input as Item);

    match item {
        Item::Mod(mut mod_item) => {
            // we are only interested in the items.
            let (_, items) = mod_item.content.as_mut().expect("mod is empty");

            // We will throw away all struct that have message attribute and collect some info.
            let mut message_params: Vec<(Ident, Generics, Type, bool)> = Vec::new();
            // We collect attributes separately as they would apply to the final enum.
            let mut attributes: Vec<Attribute> = Vec::new();
            // We extract the actor's ident string and use it generate message enum struct ident.
            let mut actor_ident_str = String::new();

            for item in items.iter_mut() {
                match item {
                    Item::Struct(struct_item) => {
                        // before we throw them we collect all the type, field and message's return type
                        // attributes other than message are collected as well.
                        if let Some(attr) = is_ident(&struct_item.attrs, "message") {
                            let mut test: String = attr
                                .tokens
                                .to_string()
                                .split('=')
                                .collect::<Vec<&str>>()
                                .pop()
                                .expect("#[message(result = \"T\")] is missing")
                                .chars()
                                .filter(|char| char != &'\"' && char != &' ')
                                .collect();

                            test.pop();

                            let is_blocking = test.contains("blocking");

                            if is_blocking {
                                for _i in 0..9 {
                                    test.pop();
                                }
                            }

                            let result_typ =
                                syn::parse_str::<syn::Type>(&test).unwrap_or_else(|_| {
                                    panic!("Failed parsing string: {} to type", test)
                                });

                            message_params.push((
                                struct_item.ident.clone(),
                                struct_item.generics.clone(),
                                result_typ,
                                is_blocking,
                            ));

                            // ToDo: We are doing extra work here and collect the message attribute too.
                            attributes.extend(struct_item.attrs.iter().cloned());

                            // remove all attribute for message type.
                            (*struct_item).attrs = vec![];
                        }

                        if let Some(_attr) = is_ident(&struct_item.attrs, "actor") {
                            actor_ident_str = struct_item.ident.to_string();
                        }
                    }
                    Item::Type(type_item) => {
                        // before we throw them we collect all the type, field and message's return type
                        // attributes other than message are collected as well.
                        if let Some(attr) = is_ident(&type_item.attrs, "message") {
                            let mut test: String = attr
                                .tokens
                                .to_string()
                                .split('=')
                                .collect::<Vec<&str>>()
                                .pop()
                                .expect("#[message(result = \"T\")] is missing")
                                .chars()
                                .filter(|char| char != &'\"' && char != &' ')
                                .collect();

                            test.pop();

                            let is_blocking = test.contains("blocking");

                            if is_blocking {
                                for _i in 0..9 {
                                    test.pop();
                                }
                            }

                            let result_typ =
                                syn::parse_str::<syn::Type>(&test).unwrap_or_else(|_| {
                                    panic!("Failed parsing string: {} to type", test)
                                });

                            message_params.push((
                                type_item.ident.clone(),
                                type_item.generics.clone(),
                                result_typ,
                                is_blocking,
                            ));

                            // ToDo: We are doing extra work here and collect the message attribute too.
                            attributes.extend(type_item.attrs.iter().cloned());

                            // remove all attribute for message type.
                            (*type_item).attrs = vec![];
                        }

                        if let Some(_attr) = is_ident(&type_item.attrs, "actor") {
                            actor_ident_str = type_item.ident.to_string();
                        }
                    }
                    _ => (),
                }
            }

            // remove all message attributes
            attributes = attributes
                .into_iter()
                .filter(|attr| {
                    attr.path
                        .segments
                        .first()
                        .map(|seg| {
                            let PathSegment { ident, .. } = seg;
                            ident.to_string().as_str() != "message"
                        })
                        .unwrap_or(true)
                })
                .collect();

            let message_enum_ident =
                Ident::new(&format!("{}Message", actor_ident_str), Span::call_site());

            // we pack the message_params into an enum.
            let mut message_enum = ItemEnum {
                attrs: attributes,
                vis: Visibility::Public(VisPublic {
                    pub_token: Default::default(),
                }),
                enum_token: Default::default(),
                ident: message_enum_ident.clone(),
                generics: Default::default(),
                brace_token: Default::default(),
                variants: Default::default(),
            };

            let result_enum_ident =
                Ident::new(&format!("{}Result", actor_ident_str), Span::call_site());

            // pack the result type into an enum too.
            let mut result_enum = ItemEnum {
                attrs: vec![],
                vis: Visibility::Public(VisPublic {
                    pub_token: Default::default(),
                }),
                enum_token: Default::default(),
                ident: result_enum_ident.clone(),
                generics: Default::default(),
                brace_token: Default::default(),
                variants: Default::default(),
            };

            // construct a type for message enum which will be used for From trait.
            let message_enum_type =
                Type::Path(type_path_from_idents(vec![message_enum_ident.clone()]));

            // ToDo: for now we ignore all generic params for message.
            for (message_ident, _generics, result_type, is_blocking) in
                message_params.iter().cloned()
            {
                // construct a message's type path firstly we would use it multiple times later
                let message_type_path = type_path_from_idents(vec![message_ident.clone()]);

                // construct message enum's new variant from message ident and type path
                let mut unnamed = FieldsUnnamed {
                    paren_token: Default::default(),
                    unnamed: Default::default(),
                };
                unnamed.unnamed.push(Field {
                    attrs: vec![],
                    vis: Visibility::Inherited,
                    ident: None,
                    colon_token: None,
                    ty: Type::Path(message_type_path.clone()),
                });
                message_enum.variants.push(Variant {
                    attrs: vec![],
                    ident: message_ident.clone(),
                    fields: Fields::Unnamed(unnamed),
                    discriminant: None,
                });

                // construct message result enum's new variant from message result type
                // If we are handling a blocking message then we have to transform the result type
                // to Result<message result type, ActixSendError>
                let mut unnamed = FieldsUnnamed {
                    paren_token: Default::default(),
                    unnamed: Default::default(),
                };
                let ty = if is_blocking {
                    let mut tp = TypePath {
                        qself: None,
                        path: Path {
                            leading_colon: None,
                            segments: Default::default(),
                        },
                    };

                    let mut args = AngleBracketedGenericArguments {
                        colon2_token: None,
                        lt_token: Default::default(),
                        args: Default::default(),
                        gt_token: Default::default(),
                    };

                    args.args.push(GenericArgument::Type(result_type.clone()));
                    args.args
                        .push(GenericArgument::Type(Type::Path(type_path_from_idents(
                            vec![Ident::new("ActixSendError", Span::call_site())],
                        ))));

                    tp.path.segments.push(PathSegment {
                        ident: Ident::new("Result", Span::call_site()),
                        arguments: PathArguments::AngleBracketed(args),
                    });

                    Type::Path(tp)
                } else {
                    result_type.clone()
                };

                unnamed.unnamed.push(Field {
                    attrs: vec![],
                    vis: Visibility::Inherited,
                    ident: None,
                    colon_token: None,
                    ty,
                });
                result_enum.variants.push(Variant {
                    attrs: vec![],
                    ident: message_ident.clone(),
                    fields: Fields::Unnamed(unnamed),
                    discriminant: None,
                });

                // impl From<Message> for ActorMessage
                // ToDo: we construct this impl item with every iteration now which is not necessary.
                let impl_item = from_trait(
                    message_type_path.clone(),
                    message_ident.clone(),
                    message_enum_ident.clone(),
                    message_enum_type.clone(),
                );

                // impl actix_send::ParseResult<ActorResult> for original Message::Result(before transformed to enum)
                let result_enum_type =
                    Type::Path(type_path_from_idents(vec![result_enum_ident.clone()]));

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

                bracket
                    .args
                    .push(GenericArgument::Type(result_enum_type.clone()));

                path.segments.push(PathSegment {
                    ident: Ident::new("MapResult", Span::call_site()),
                    arguments: PathArguments::AngleBracketed(bracket),
                });

                let mut expr_path = Path {
                    leading_colon: None,
                    segments: Default::default(),
                };

                expr_path.segments.push(PathSegment {
                    ident: result_enum_ident.clone(),
                    arguments: Default::default(),
                });

                let mut expr_call = ExprCall {
                    attrs: vec![],
                    func: Box::new(Expr::Path(ExprPath {
                        attrs: vec![],
                        qself: None,
                        path: expr_path,
                    })),
                    paren_token: Default::default(),
                    args: Default::default(),
                };

                expr_call.args.push(Expr::Path(ExprPath {
                    attrs: vec![],
                    qself: None,
                    path: path_from_ident_str(vec!["result"]),
                }));

                let mut arms = Vec::new();

                let mut arm_path = Path {
                    leading_colon: None,
                    segments: Default::default(),
                };

                arm_path.segments.push(PathSegment {
                    ident: result_enum_ident.clone(),
                    arguments: Default::default(),
                });

                arm_path.segments.push(PathSegment {
                    ident: message_ident.clone(),
                    arguments: Default::default(),
                });

                let mut pat = PatTuple {
                    attrs: vec![],
                    paren_token: Default::default(),
                    elems: Default::default(),
                };

                let result_ident = Ident::new("result", Span::call_site());

                pat.elems.push(Pat::Ident(PatIdent {
                    attrs: vec![],
                    by_ref: None,
                    mutability: None,
                    ident: result_ident.clone(),
                    subpat: None,
                }));

                let mut result_path = Path {
                    leading_colon: None,
                    segments: Default::default(),
                };

                let mut path_args = ParenthesizedGenericArguments {
                    paren_token: Default::default(),
                    inputs: Default::default(),
                    output: ReturnType::Default,
                };

                path_args.inputs.push(Type::Path(type_path_from_idents(
                    vec![result_ident.clone()],
                )));

                // If we are handling a blocking message then the result_path doesn't have to be
                // wrapped in Ok().
                if is_blocking {
                    result_path.segments.push(PathSegment {
                        ident: result_ident,
                        arguments: Default::default(),
                    });
                } else {
                    result_path.segments.push(PathSegment {
                        ident: Ident::new("Ok", Span::call_site()),
                        arguments: PathArguments::Parenthesized(path_args),
                    });
                }

                arms.push(Arm {
                    attrs: vec![],
                    pat: Pat::TupleStruct(PatTupleStruct {
                        attrs: vec![],
                        path: arm_path,
                        pat,
                    }),
                    guard: None,
                    fat_arrow_token: Default::default(),
                    body: Box::new(Expr::Path(ExprPath {
                        attrs: vec![],
                        qself: None,
                        path: result_path,
                    })),
                    comma: Some(Default::default()),
                });

                arms.push(Arm {
                    attrs: vec![],
                    pat: Pat::Wild(PatWild {
                        attrs: vec![],
                        underscore_token: Default::default(),
                    }),
                    guard: None,
                    fat_arrow_token: Default::default(),
                    body: Box::new(Expr::Macro(ExprMacro {
                        attrs: vec![],
                        mac: Macro {
                            path: path_from_ident_str(vec!["unreachable"]),
                            bang_token: Default::default(),
                            delimiter: MacroDelimiter::Paren(Paren {
                                span: Span::call_site(),
                            }),
                            tokens: Default::default(),
                        },
                    })),
                    comma: None,
                });

                let mut result_path = Path {
                    leading_colon: None,
                    segments: Default::default(),
                };

                let mut bracket = AngleBracketedGenericArguments {
                    colon2_token: None,
                    lt_token: Default::default(),
                    args: Default::default(),
                    gt_token: Default::default(),
                };

                bracket
                    .args
                    .push(GenericArgument::Type(Type::Path(type_path_from_idents(
                        vec![
                            Ident::new("Self", Span::call_site()),
                            Ident::new("Output", Span::call_site()),
                        ],
                    ))));

                bracket
                    .args
                    .push(GenericArgument::Type(Type::Path(type_path_from_idents(
                        vec![Ident::new("ActixSendError", Span::call_site())],
                    ))));

                result_path.segments.push(PathSegment {
                    ident: Ident::new("Result", Span::call_site()),
                    arguments: PathArguments::AngleBracketed(bracket),
                });

                let mut method = ImplItemMethod {
                    attrs: vec![],
                    vis: Visibility::Inherited,
                    defaultness: None,
                    sig: Signature {
                        constness: None,
                        asyncness: None,
                        unsafety: None,
                        abi: None,
                        fn_token: Default::default(),
                        ident: Ident::new("map", Span::call_site()),
                        generics: Default::default(),
                        paren_token: Default::default(),
                        inputs: Default::default(),
                        variadic: None,
                        output: ReturnType::Type(
                            Default::default(),
                            Box::new(Type::Path(TypePath {
                                qself: None,
                                path: result_path,
                            })),
                        ),
                    },
                    block: Block {
                        brace_token: Default::default(),
                        stmts: vec![Stmt::Expr(Expr::Match(ExprMatch {
                            attrs: vec![],
                            match_token: Default::default(),
                            expr: Box::new(Expr::Path(ExprPath {
                                attrs: vec![],
                                qself: None,
                                path: path_from_ident_str(vec!["msg"]),
                            })),
                            brace_token: Default::default(),
                            arms,
                        }))],
                    },
                };

                method.sig.inputs.push(FnArg::Typed(PatType {
                    attrs: vec![],
                    pat: Box::new(Pat::Ident(PatIdent {
                        attrs: vec![],
                        by_ref: None,
                        mutability: None,
                        ident: Ident::new("msg", Span::call_site()),
                        subpat: None,
                    })),
                    colon_token: Default::default(),
                    ty: Box::new(result_enum_type.clone()),
                }));

                let impl_type = ImplItemType {
                    attrs: vec![],
                    vis: Visibility::Inherited,
                    defaultness: None,
                    type_token: Default::default(),
                    ident: Ident::new("Output", Span::call_site()),
                    generics: Default::default(),
                    eq_token: Default::default(),
                    ty: result_type,
                    semi_token: Default::default(),
                };

                let impl_item2 = Item::Impl(ItemImpl {
                    attrs: vec![],
                    defaultness: None,
                    unsafety: None,
                    impl_token: Default::default(),
                    generics: Default::default(),
                    trait_: Some((None, path, Default::default())),
                    self_ty: Box::new(Type::Path(message_type_path.clone())),
                    brace_token: Default::default(),
                    items: vec![ImplItem::Type(impl_type), ImplItem::Method(method)],
                });

                items.push(impl_item);
                items.push(impl_item2);
            }

            items.push(Item::Enum(message_enum));
            items.push(Item::Enum(result_enum));

            let handle_methods = items
                .iter()
                .filter_map(|item| {
                    let item_impl = match item {
                        Item::Impl(i) => i,
                        _ => return None,
                    };
                    let _attr = is_ident(&item_impl.attrs, "handler")?;
                    // ToDo: we should check the actor identity in future if we want to handle multiple actors in one module
                    let impl_item = item_impl.items.first()?;

                    match impl_item {
                        ImplItem::Method(method) => Some(method),
                        _ => None,
                    }
                })
                .map(|method| {
                    // We want to collect the second arg of the inputs(The message ident)
                    // We would also want to collect the statements
                    let mut args = method.sig.inputs.iter();
                    args.next();

                    let (arg_ident, ident) = args
                        .next()
                        .map(|arg| {
                            if let FnArg::Typed(pat) = arg {
                                if let Type::Path(TypePath { path, .. }) = pat.ty.as_ref() {
                                    let seg = path.segments.first()?;

                                    let arg_ident = match pat.pat.as_ref() {
                                        Pat::Ident(ident) => ident.ident.clone(),
                                        _ => Ident::new("_msg", Span::call_site()),
                                    };

                                    return Some((arg_ident, &seg.ident));
                                }
                            }
                            None
                        })
                        .expect("handle method must have a legit TypePath for Message type")
                        .expect("handle method must have a argument as msg: MessageType");

                    (arg_ident, ident.clone(), method.block.stmts.clone())
                })
                .collect::<Vec<(Ident, Ident, Vec<Stmt>)>>();

            // ToDo: We are doing extra work removing all the #[handler] impls
            *items = items
                .iter()
                .filter(|item| {
                    let item_impl = match item {
                        Item::Impl(i) => i,
                        _ => return true,
                    };
                    is_ident(&item_impl.attrs, "handler").is_none()
                })
                .cloned()
                .collect::<Vec<Item>>();

            // We generate a real handle method for ActorMessage enum and pattern match the handle async functions.
            // The return type of this handle method would be ActorMessageResult enum.
            let actor_ident = Ident::new(actor_ident_str.as_str(), Span::call_site());

            let mut inputs = Punctuated::new();
            inputs.push(FnArg::Receiver(Receiver {
                attrs: vec![],
                reference: Some(Default::default()),
                mutability: Some(Default::default()),
                self_token: Default::default(),
            }));
            inputs.push(FnArg::Typed(PatType {
                attrs: vec![],
                pat: Box::new(Pat::Ident(PatIdent {
                    attrs: vec![],
                    by_ref: None,
                    mutability: None,
                    ident: Ident::new("msg", Span::call_site()),
                    subpat: None,
                })),
                colon_token: Default::default(),
                ty: Box::new(message_enum_type),
            }));

            let mut path = Path {
                leading_colon: None,
                segments: Default::default(),
            };

            path.segments.push(PathSegment {
                ident: message_enum_ident,
                arguments: Default::default(),
            });

            // We just throw the statements of handle method for every type of message into the final handle method's enum variants.

            let arms = message_params
                .into_iter()
                .map(|(message_ident, _, _, is_blocking)| {
                    let mut path = path.clone();

                    path.segments.push(PathSegment {
                        ident: message_ident.clone(),
                        arguments: Default::default(),
                    });

                    let mut pat = PatTuple {
                        attrs: vec![],
                        paren_token: Default::default(),
                        elems: Default::default(),
                    };

                    let ident = handle_methods
                        .iter()
                        .find_map(|(arg_ident, msg_ident, _)| {
                            if msg_ident == &message_ident {
                                Some(arg_ident.clone())
                            } else {
                                None
                            }
                        })
                        .unwrap();

                    pat.elems.push(Pat::Ident(PatIdent {
                        attrs: vec![],
                        by_ref: None,
                        mutability: None,
                        ident,
                        subpat: None,
                    }));

                    let panic = format!(
                        "We can not find Handler::handle method for message type: {}",
                        &message_ident
                    );

                    let stmts = handle_methods
                        .iter()
                        .find_map(|(_, ident, stmts)| {
                            if ident == &message_ident {
                                Some(stmts.clone())
                            } else {
                                None
                            }
                        })
                        .expect(&panic);

                    // If the message have blocking attribute we wrap the method in runtime::spawn_blocking
                    let stmt1 = if is_blocking {
                        let mut expr_call = ExprCall {
                            attrs: vec![],
                            func: Box::new(Expr::Path(ExprPath {
                                attrs: vec![],
                                qself: None,
                                path: path_from_ident_str(vec!["actix_send_blocking"]),
                            })),
                            paren_token: Default::default(),
                            args: Default::default(),
                        };

                        let closure = ExprClosure {
                            attrs: vec![],
                            asyncness: None,
                            movability: None,
                            capture: Some(Default::default()),
                            or1_token: Default::default(),
                            inputs: Default::default(),
                            or2_token: Default::default(),
                            output: ReturnType::Default,
                            body: Box::new(Expr::Block(ExprBlock {
                                attrs: vec![],
                                label: None,
                                block: Block {
                                    brace_token: Default::default(),
                                    stmts,
                                },
                            })),
                        };

                        expr_call.args.push(Expr::Closure(closure));

                        Stmt::Local(Local {
                            attrs: vec![],
                            let_token: Default::default(),
                            pat: Pat::Ident(PatIdent {
                                attrs: vec![],
                                by_ref: None,
                                mutability: None,
                                ident: Ident::new("result", Span::call_site()),
                                subpat: None,
                            }),
                            init: Some((Default::default(), Box::new(Expr::Call(expr_call)))),
                            semi_token: Default::default(),
                        })
                    } else {
                        Stmt::Local(Local {
                            attrs: vec![],
                            let_token: Default::default(),
                            pat: Pat::Ident(PatIdent {
                                attrs: vec![],
                                by_ref: None,
                                mutability: None,
                                ident: Ident::new("result", Span::call_site()),
                                subpat: None,
                            }),
                            init: Some((
                                Default::default(),
                                Box::new(Expr::Async(ExprAsync {
                                    attrs: vec![],
                                    async_token: Default::default(),
                                    capture: Some(Default::default()),
                                    block: Block {
                                        brace_token: Default::default(),
                                        stmts,
                                    },
                                })),
                            )),
                            semi_token: Default::default(),
                        })
                    };

                    let mut path_stmt2 = Path {
                        leading_colon: None,
                        segments: Default::default(),
                    };

                    path_stmt2.segments.push(PathSegment {
                        ident: result_enum_ident.clone(),
                        arguments: PathArguments::None,
                    });

                    path_stmt2.segments.push(PathSegment {
                        ident: message_ident,
                        arguments: PathArguments::None,
                    });

                    let mut expr_call = ExprCall {
                        attrs: vec![],
                        func: Box::new(Expr::Path(ExprPath {
                            attrs: vec![],
                            qself: None,
                            path: path_stmt2,
                        })),
                        paren_token: Default::default(),
                        args: Default::default(),
                    };

                    expr_call.args.push(Expr::Await(ExprAwait {
                        attrs: vec![],
                        base: Box::new(Expr::Path(ExprPath {
                            attrs: vec![],
                            qself: None,
                            path: path_from_ident_str(vec!["result"]),
                        })),
                        dot_token: Default::default(),
                        await_token: Default::default(),
                    }));

                    let stmt2 = Stmt::Expr(Expr::Call(expr_call));

                    Arm {
                        attrs: vec![],
                        pat: Pat::TupleStruct(PatTupleStruct {
                            attrs: vec![],
                            path,
                            pat,
                        }),
                        guard: None,
                        fat_arrow_token: Default::default(),
                        body: Box::new(Expr::Block(ExprBlock {
                            attrs: vec![],
                            label: None,
                            block: Block {
                                brace_token: Default::default(),
                                stmts: vec![stmt1, stmt2],
                            },
                        })),
                        comma: Some(Default::default()),
                    }
                })
                .collect();

            let handle = Item::Impl(ItemImpl {
                attrs: vec![attr_from_ident_str(vec!["handler"])],
                defaultness: None,
                unsafety: None,
                impl_token: Default::default(),
                generics: Default::default(),
                trait_: Some((
                    None,
                    path_from_ident_str(vec!["Handler"]),
                    Default::default(),
                )),
                self_ty: Box::new(Type::Path(type_path_from_idents(vec![actor_ident.clone()]))),
                brace_token: Default::default(),
                items: vec![ImplItem::Method(ImplItemMethod {
                    attrs: vec![],
                    vis: Visibility::Inherited,
                    defaultness: None,
                    sig: Signature {
                        constness: None,
                        asyncness: Some(Default::default()),
                        unsafety: None,
                        abi: None,
                        fn_token: Default::default(),
                        ident: Ident::new("handle", Span::call_site()),
                        generics: Default::default(),
                        paren_token: Default::default(),
                        inputs,
                        variadic: None,
                        output: ReturnType::Type(
                            Default::default(),
                            Box::new(Type::Path(type_path_from_idents(vec![result_enum_ident]))),
                        ),
                    },
                    block: Block {
                        brace_token: Default::default(),
                        stmts: vec![Stmt::Expr(Expr::Match(ExprMatch {
                            attrs: vec![],
                            match_token: Default::default(),
                            expr: Box::new(Expr::Path(ExprPath {
                                attrs: vec![],
                                qself: None,
                                path: path_from_ident_str(vec!["msg"]),
                            })),
                            brace_token: Default::default(),
                            arms,
                        }))],
                    },
                })],
            });

            items.push(handle);

            let expand = quote! {
                #mod_item

                // ToDo: Fix this dummy implementation
                impl #actor_ident {
                    pub fn __on_start(&mut self) {}
                    pub fn __on_stop(&mut self) {}
                }
            };

            expand.into()
        }
        _ => unreachable!("#[actor_with_messages] must be used on a mod."),
    }
}

#[proc_macro_attribute]
pub fn handler_v2(_meta: TokenStream, input: TokenStream) -> TokenStream {
    let item = syn::parse_macro_input!(input as Item);

    match item {
        Item::Impl(mut impl_item) => {
            // get actor ident.
            let actor_ident = match impl_item.self_ty.as_ref() {
                Type::Path(ty_path) => ty_path.path.get_ident().unwrap(),
                _ => unreachable!("#[handler_v2] must be used on impl ActorTypePath."),
            };

            let mut actor_info = ActorInfo::new(actor_ident);

            // find method that with on_start attribute and remove the item.
            let mut on_start: Option<ImplItemMethod> = None;
            let mut on_stop: Option<ImplItemMethod> = None;

            impl_item.items.retain(|f| match f {
                ImplItem::Method(method) => {
                    if is_ident(&method.attrs, "on_start").is_some() {
                        on_start = Some(method.clone());
                        return false;
                    }
                    if is_ident(&method.attrs, "on_stop").is_some() {
                        on_stop = Some(method.clone());
                        return false;
                    }
                    true
                }
                _ => false,
            });

            let mut on_start_method = ImplItemMethod {
                attrs: vec![],
                vis: Visibility::Public(VisPublic {
                    pub_token: Default::default(),
                }),
                defaultness: None,
                sig: Signature {
                    constness: None,
                    asyncness: None,
                    unsafety: None,
                    abi: None,
                    fn_token: Default::default(),
                    ident: Ident::new("__on_start", Span::call_site()),
                    generics: Default::default(),
                    paren_token: Default::default(),
                    inputs: Default::default(),
                    variadic: None,
                    output: ReturnType::Default,
                },
                block: Block {
                    brace_token: Default::default(),
                    stmts: vec![],
                },
            };

            let mut on_stop_method = ImplItemMethod {
                attrs: vec![],
                vis: Visibility::Public(VisPublic {
                    pub_token: Default::default(),
                }),
                defaultness: None,
                sig: Signature {
                    constness: None,
                    asyncness: None,
                    unsafety: None,
                    abi: None,
                    fn_token: Default::default(),
                    ident: Ident::new("__on_stop", Span::call_site()),
                    generics: Default::default(),
                    paren_token: Default::default(),
                    inputs: Default::default(),
                    variadic: None,
                    output: ReturnType::Default,
                },
                block: Block {
                    brace_token: Default::default(),
                    stmts: vec![],
                },
            };

            let self_arg = FnArg::Receiver(Receiver {
                attrs: vec![],
                reference: Some(Default::default()),
                mutability: Some(Default::default()),
                self_token: Default::default(),
            });

            on_start_method.sig.inputs.push(self_arg.clone());
            on_stop_method.sig.inputs.push(self_arg.clone());

            // construct on_start and on_stop impl if there is any.

            if let Some(mut method) = on_start.take() {
                method.attrs.retain(|attr| {
                    attr.path
                        .get_ident()
                        .map(|ident| ident.to_string().as_str() != "on_start")
                        .unwrap_or(true)
                });
                on_start_method.attrs = method.attrs;
                on_start_method.block.stmts = method.block.stmts;
            }

            if let Some(mut method) = on_stop.take() {
                method.attrs.retain(|attr| {
                    attr.path
                        .get_ident()
                        .map(|ident| ident.to_string().as_str() != "on_stop")
                        .unwrap_or(true)
                });

                on_stop_method.attrs = method.attrs;
                on_stop_method.block.stmts = method.block.stmts;
            }

            let on_start_stop = Item::Impl(ItemImpl {
                attrs: vec![],
                defaultness: None,
                unsafety: None,
                impl_token: Default::default(),
                generics: Default::default(),
                trait_: None,
                self_ty: impl_item.self_ty.clone(),
                brace_token: Default::default(),
                items: vec![
                    ImplItem::Method(on_start_method),
                    ImplItem::Method(on_stop_method),
                ],
            });

            let handle_info = impl_item
                .items
                .iter()
                .filter_map(|item| match item {
                    ImplItem::Method(method) => Some(HandleMethodInfo::new(method)),
                    _ => None,
                })
                .collect::<Vec<HandleMethodInfo>>();

            actor_info
                .enum_variants(&handle_info)
                .from_trait(&handle_info)
                .map_result_trait(&handle_info)
                .handler_trait(&handle_info);
            // .send_method_wrapper(&handle_info);

            let message_enum = &actor_info.message_enum;
            let result_enum = &actor_info.result_enum;
            let items = &actor_info.items;

            let expand = quote! {
                #on_start_stop

                #message_enum
                #result_enum

                #(#items)*
            };

            expand.into()
        }
        _ => unreachable!("#[actor_with_messages] must be used on a mod."),
    }
}

fn from_trait(
    source_type_path: TypePath,
    source_ident: Ident,
    message_enum_ident: Ident,
    message_enum_type: Type,
) -> Item {
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

    bracket
        .args
        .push(GenericArgument::Type(Type::Path(source_type_path.clone())));

    path.segments.push(PathSegment {
        ident: Ident::new("From", Span::call_site()),
        arguments: PathArguments::AngleBracketed(bracket),
    });

    let mut expr_path = Path {
        leading_colon: None,
        segments: Default::default(),
    };

    expr_path.segments.push(PathSegment {
        ident: message_enum_ident.clone(),
        arguments: Default::default(),
    });

    expr_path.segments.push(PathSegment {
        ident: source_ident,
        arguments: Default::default(),
    });

    let mut expr_call = ExprCall {
        attrs: vec![],
        func: Box::new(Expr::Path(ExprPath {
            attrs: vec![],
            qself: None,
            path: expr_path,
        })),
        paren_token: Default::default(),
        args: Default::default(),
    };

    expr_call.args.push(Expr::Path(ExprPath {
        attrs: vec![],
        qself: None,
        path: path_from_ident_str(vec!["msg"]),
    }));

    let mut method = ImplItemMethod {
        attrs: vec![],
        vis: Visibility::Inherited,
        defaultness: None,
        sig: Signature {
            constness: None,
            asyncness: None,
            unsafety: None,
            abi: None,
            fn_token: Default::default(),
            ident: Ident::new("from", Span::call_site()),
            generics: Default::default(),
            paren_token: Default::default(),
            inputs: Default::default(),
            variadic: None,
            output: ReturnType::Type(
                Default::default(),
                Box::new(Type::Path(type_path_from_idents(vec![message_enum_ident]))),
            ),
        },
        block: Block {
            brace_token: Default::default(),
            stmts: vec![Stmt::Expr(Expr::Call(expr_call))],
        },
    };

    method.sig.inputs.push(FnArg::Typed(PatType {
        attrs: vec![],
        pat: Box::new(Pat::Ident(PatIdent {
            attrs: vec![],
            by_ref: None,
            mutability: None,
            ident: Ident::new("msg", Span::call_site()),
            subpat: None,
        })),
        colon_token: Default::default(),
        ty: Box::new(Type::Path(source_type_path)),
    }));

    Item::Impl(ItemImpl {
        attrs: vec![],
        defaultness: None,
        unsafety: None,
        impl_token: Default::default(),
        generics: Default::default(),
        trait_: Some((None, path, Default::default())),
        self_ty: Box::new(message_enum_type),
        brace_token: Default::default(),
        items: vec![ImplItem::Method(method)],
    })
}

fn is_ident<'a>(attrs: &'a [Attribute], ident_str: &str) -> Option<&'a Attribute> {
    attrs.iter().find(|attr| {
        attr.path
            .segments
            .first()
            .map(|seg| {
                let PathSegment { ident, .. } = seg;
                ident.to_string().as_str() == ident_str
            })
            .unwrap_or(false)
    })
}

fn static_message(item: Item, result: Type) -> TokenStream {
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

// helper function for generating attribute.
fn attr_from_ident_str(ident_str: Vec<&str>) -> Attribute {
    Attribute {
        pound_token: Default::default(),
        style: AttrStyle::Outer,
        bracket_token: Default::default(),
        path: path_from_ident_str(ident_str),
        tokens: Default::default(),
    }
}

// helper function for generating path.
fn path_from_ident_str(ident_str: Vec<&str>) -> Path {
    let mut path = Path {
        leading_colon: None,
        segments: Default::default(),
    };

    for ident_str in ident_str.into_iter() {
        path.segments.push(PathSegment {
            ident: Ident::new(ident_str, Span::call_site()),
            arguments: Default::default(),
        });
    }

    path
}

fn type_path_from_idents(idents: Vec<Ident>) -> TypePath {
    let mut path = Path {
        leading_colon: None,
        segments: Default::default(),
    };

    for ident in idents.into_iter() {
        path.segments.push(PathSegment {
            ident,
            arguments: Default::default(),
        })
    }

    TypePath { qself: None, path }
}
