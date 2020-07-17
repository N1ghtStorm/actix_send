use syn::{
    export::Span, punctuated::Punctuated, token::Paren, AngleBracketedGenericArguments, Arm, Block,
    Expr, ExprAsync, ExprAwait, ExprBlock, ExprCall, ExprClosure, ExprMacro, ExprMatch, ExprPath,
    Field, Fields, FieldsUnnamed, FnArg, GenericArgument, Ident, ImplItem, ImplItemMethod,
    ImplItemType, Item, ItemEnum, ItemImpl, Local, Macro, MacroDelimiter,
    ParenthesizedGenericArguments, Pat, PatIdent, PatTuple, PatTupleStruct, PatType, PatWild, Path,
    PathArguments, PathSegment, Receiver, ReturnType, Signature, Stmt, Type, TypePath, TypeTuple,
    Variant, VisPublic, Visibility,
};

use crate::{attr_from_ident_str, path_from_ident_str, type_path_from_idents};

// A struct contains Actor specific info.
pub(crate) struct ActorInfo<'a> {
    pub(crate) ident: &'a Ident,
    pub(crate) message_enum_ident: Ident,
    pub(crate) message_enum_type: Type,
    pub(crate) result_enum_ident: Ident,
    pub(crate) message_enum: ItemEnum,
    pub(crate) result_enum: ItemEnum,
    pub(crate) items: Vec<Item>,
}

impl<'a> ActorInfo<'a> {
    pub(crate) fn new(ident: &'a Ident) -> Self {
        let ident_string = ident.to_string();

        let message_enum_ident = Ident::new(
            &format!("{}Message", ident_string.as_str()),
            Span::call_site(),
        );

        // we pack the message_params into an enum.
        let message_enum = ItemEnum {
            attrs: Default::default(),
            vis: Visibility::Public(VisPublic {
                pub_token: Default::default(),
            }),
            enum_token: Default::default(),
            ident: message_enum_ident.clone(),
            generics: Default::default(),
            brace_token: Default::default(),
            variants: Default::default(),
        };

        let result_enum_ident = Ident::new(
            &format!("{}Result", ident_string.as_str()),
            Span::call_site(),
        );

        // pack the result type into an enum too.
        let result_enum = ItemEnum {
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

        Self {
            ident,
            message_enum_ident: message_enum_ident.clone(),
            message_enum_type: Type::Path(type_path_from_idents(vec![message_enum_ident])),
            message_enum,
            result_enum_ident,
            result_enum,
            items: Vec::new(),
        }
    }

    // generate message enum and result enum variants.
    pub(crate) fn enum_variants(&mut self, handle_info: &[HandleMethodInfo]) -> &mut Self {
        for handle in handle_info.iter() {
            let message_type_path = handle.message_type_path;

            let message_ident = message_type_path.path.get_ident().unwrap();

            let default_result_type = Type::Tuple(TypeTuple {
                paren_token: Default::default(),
                elems: Default::default(),
            });
            let result_type = handle
                .message_return_type
                .map(Clone::clone)
                .unwrap_or(default_result_type);

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
            self.message_enum.variants.push(Variant {
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
            let ty = if handle.is_async {
                result_type.clone()
            } else {
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
            };

            unnamed.unnamed.push(Field {
                attrs: vec![],
                vis: Visibility::Inherited,
                ident: None,
                colon_token: None,
                ty,
            });
            self.result_enum.variants.push(Variant {
                attrs: vec![],
                ident: message_ident.clone(),
                fields: Fields::Unnamed(unnamed),
                discriminant: None,
            });
        }

        self
    }

    // Generate From<Message> for ActorMessage
    pub(crate) fn from_trait(&mut self, handle_info: &[HandleMethodInfo]) -> &mut Self {
        let message_enum_ident = &self.message_enum_ident;
        let message_enum_type = &self.message_enum_type;

        for handle in handle_info.iter() {
            let message_type_path = handle.message_type_path;
            let message_ident = message_type_path.path.get_ident().unwrap();

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
                .push(GenericArgument::Type(Type::Path(message_type_path.clone())));

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
                ident: message_ident.clone(),
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
                        Box::new(message_enum_type.clone()),
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
                ty: Box::new(Type::Path(message_type_path.clone())),
            }));

            let from = Item::Impl(ItemImpl {
                attrs: vec![],
                defaultness: None,
                unsafety: None,
                impl_token: Default::default(),
                generics: Default::default(),
                trait_: Some((None, path, Default::default())),
                self_ty: Box::new(message_enum_type.clone()),
                brace_token: Default::default(),
                items: vec![ImplItem::Method(method)],
            });

            self.items.push(from);
        }

        self
    }

    // impl actix_send::ParseResult<ActorResult> for original Message::Result(before transformed to enum)
    pub(crate) fn map_result_trait(&mut self, handle_info: &[HandleMethodInfo]) -> &mut Self {
        let result_enum_type =
            Type::Path(type_path_from_idents(vec![self.result_enum_ident.clone()]));
        let result_enum_ident = &self.result_enum_ident;

        for handle in handle_info.iter() {
            let message_ident = handle.message_type_path.path.get_ident().unwrap();
            let message_type_path = handle.message_type_path;

            let default_result_type = Type::Tuple(TypeTuple {
                paren_token: Default::default(),
                elems: Default::default(),
            });
            let result_type = handle
                .message_return_type
                .map(Clone::clone)
                .unwrap_or(default_result_type);

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
            if handle.is_async {
                result_path.segments.push(PathSegment {
                    ident: Ident::new("Ok", Span::call_site()),
                    arguments: PathArguments::Parenthesized(path_args),
                });
            } else {
                result_path.segments.push(PathSegment {
                    ident: result_ident,
                    arguments: Default::default(),
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

            let impl_item = Item::Impl(ItemImpl {
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

            self.items.push(impl_item);
        }

        self
    }

    // We generate a real handle method for ActorMessage enum and pattern match the handle async functions.
    // The return type of this handle method would be ActorMessageResult enum.
    pub(crate) fn handler_trait(&mut self, handle_info: &[HandleMethodInfo]) -> &mut Self {
        let actor_ident = self.ident;

        let message_enum_type = self.message_enum_type.clone();
        let message_enum_ident = self.message_enum_ident.clone();
        let result_enum_ident = &self.result_enum_ident;

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

        let arms = handle_info
            .iter()
            .map(|handle| {
                let message_ident = handle.message_type_path.path.get_ident().unwrap();

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

                pat.elems.push(Pat::Ident(PatIdent {
                    attrs: vec![],
                    by_ref: None,
                    mutability: None,
                    ident: handle.message_var_ident.clone(),
                    subpat: None,
                }));

                // we construct an optional statement is we are wrapping blocking
                let stmt0 = None;
                // if !handle.is_async {
                //     // ToDo: in case async_trait changed.
                //     // *. Here we use a hack. #[async_trait] would transfer
                //     // all self identifier to _self by default so we take use of
                //     // it and map our cloned Self to _self identifier too.
                //
                //     let st = Stmt::Local(Local {
                //         attrs: vec![],
                //         let_token: Default::default(),
                //         pat: Pat::Ident(PatIdent {
                //             attrs: vec![],
                //             by_ref: None,
                //             mutability: None,
                //             ident: Ident::new("_self", Span::call_site()),
                //             subpat: None,
                //         }),
                //         init: Some((
                //             Default::default(),
                //             Box::new(Expr::MethodCall(ExprMethodCall {
                //                 attrs: vec![],
                //                 receiver: Box::new(Expr::Path(ExprPath {
                //                     attrs: vec![],
                //                     qself: None,
                //                     path: path_from_ident_str("self"),
                //                 })),
                //                 dot_token: Default::default(),
                //                 method: Ident::new("clone", Span::call_site()),
                //                 turbofish: None,
                //                 paren_token: Default::default(),
                //                 args: Default::default(),
                //             })),
                //         )),
                //         semi_token: Default::default(),
                //     });
                //     stmt0 = Some(st);
                // };

                // If the message have blocking attribute we wrap the method in runtime::spawn_blocking
                let stmt1 = if handle.is_async {
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
                                block: handle.method_block.clone(),
                            })),
                        )),
                        semi_token: Default::default(),
                    })
                } else {
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
                            block: handle.method_block.clone(),
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
                    ident: message_ident.clone(),
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
                            stmts: match stmt0 {
                                Some(stmt0) => vec![stmt0, stmt1, stmt2],
                                None => vec![stmt1, stmt2],
                            },
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
                        Box::new(Type::Path(type_path_from_idents(vec![
                            result_enum_ident.clone()
                        ]))),
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

        self.items.push(handle);

        self
    }
}

// A struct contains all the handle methods info for one actor.
pub(crate) struct HandleMethodInfo<'a> {
    message_var_ident: Ident,
    message_type_path: &'a TypePath,
    message_return_type: Option<&'a Type>,
    // method_signature: &'a Signature,
    method_block: &'a Block,
    is_async: bool,
}

impl<'a> HandleMethodInfo<'a> {
    pub(crate) fn new(method: &'a ImplItemMethod) -> Self {
        let (message_var_ident, message_type_path) = method
            .sig
            .inputs
            .iter()
            .find_map(|arg| {
                if let FnArg::Typed(PatType { ty, pat, .. }) = arg {
                    if let Type::Path(path) = ty.as_ref() {
                        let arg_ident = match pat.as_ref() {
                            Pat::Ident(ident) => ident.ident.clone(),
                            _ => Ident::new("_msg", Span::call_site()),
                        };

                        return Some((arg_ident, path));
                    }
                }
                None
            })
            .expect("FnArg missing for message type path");

        let message_return_type = match &method.sig.output {
            ReturnType::Type(_, typ) => Some(typ.as_ref()),
            _ => None,
        };

        let is_async = method.sig.asyncness.is_some();

        Self {
            message_var_ident,
            message_type_path,
            message_return_type,
            // method_signature: &method.sig,
            method_block: &method.block,
            is_async,
        }
    }
}
