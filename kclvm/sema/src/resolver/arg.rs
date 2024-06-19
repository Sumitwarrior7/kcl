use crate::resolver::Resolver;
use crate::ty::FunctionType;
use compiler_base_error::unit_type::{TypeWithUnit, UnitUsize};
use indexmap::IndexSet;
use kclvm_ast::ast;

use kclvm_ast::pos::GetPos;

use crate::ty::TypeRef;

impl<'ctx> Resolver<'ctx> {
    fn get_func_name(&mut self, func: &ast::Expr) -> String {
        let mut callee = func;
        loop {
            match callee {
                ast::Expr::Identifier(identifier) => {
                    return format!("\"{}\"", identifier.get_name());
                }
                ast::Expr::Selector(selector_expr) => {
                    return format!("\"{}\"", selector_expr.attr.node.get_name());
                }
                ast::Expr::Paren(paren_expr) => callee = &paren_expr.expr.node,
                _ => return "anonymous function".to_string(),
            }
        }
    }

    /// Do schema/function/decorator argument type check.
    pub fn do_arguments_type_check(
        &mut self,
        func: &ast::NodeRef<ast::Expr>,
        args: &'ctx [ast::NodeRef<ast::Expr>],
        kwargs: &'ctx [ast::NodeRef<ast::Keyword>],
        func_ty: &FunctionType,
    ) {
        let func_name = self.get_func_name(&func.node);
        let arg_types = self.exprs(args);
        let mut kwarg_types: Vec<(String, TypeRef)> = vec![];
        let mut check_table: IndexSet<String> = IndexSet::default();
        let mut prev_kw_pos = None;
        for kw in kwargs {
            if !kw.node.arg.node.names.is_empty() {
                let previous_pos = if let Some(prev_pos) = prev_kw_pos {
                    prev_pos
                } else {
                    kw.get_end_pos()
                };
                let arg_name = &kw.node.arg.node.names[0].node;

                if check_table.contains(arg_name) {
                    self.handler.add_compile_error_with_suggestions(
                        &format!("{} has duplicated keyword argument {}", func_name, arg_name),
                        (previous_pos, kw.get_end_pos()),
                        Some(vec![]),
                    );
                }
                check_table.insert(arg_name.to_string());
                let arg_value_type = self.expr_or_any_type(&kw.node.value);
                self.node_ty_map
                    .borrow_mut()
                    .insert(self.get_node_key(kw.id.clone()), arg_value_type.clone());
                kwarg_types.push((arg_name.to_string(), arg_value_type.clone()));
            } else {
                self.handler
                    .add_compile_error(&format!("missing argument"), kw.get_span_pos());
            }
            prev_kw_pos = Some(kw.get_end_pos());
        }
        // Do few argument count check
        if !func_ty.is_variadic {
            let mut got_count = 0;
            let mut expect_count = 0;
            for param in &func_ty.params {
                if !param.has_default {
                    expect_count += 1;
                    if check_table.contains(&param.name) {
                        got_count += 1
                    }
                }
            }
            got_count += args.len();
            if got_count < expect_count {
                self.handler.add_compile_error(
                    &format!(
                        "expected {}, found {}",
                        UnitUsize(expect_count, "positional argument".to_string())
                            .into_string_with_unit(),
                        got_count
                    ),
                    func.get_span_pos(),
                );
            }
        }
        // Do normal argument type check
        for (i, ty) in arg_types.iter().enumerate() {
            let expected_ty = match func_ty.params.get(i) {
                Some(param) => param.ty.clone(),
                None => {
                    if !func_ty.is_variadic {
                        self.handler.add_compile_error_with_suggestions(
                            &format!(
                                "{} takes {} but {} were given",
                                func_name,
                                UnitUsize(func_ty.params.len(), "positional argument".to_string())
                                    .into_string_with_unit(),
                                args.len(),
                            ),
                            args[i].get_span_pos(),
                            Some(vec![]),
                        );
                    }
                    return;
                }
            };
            self.must_assignable_to(
                ty.clone(),
                expected_ty.clone(),
                args[i].get_span_pos(),
                None,
            );

            let upgrade_schema_type = self.upgrade_dict_to_schema(
                ty.clone(),
                expected_ty.clone(),
                &args[i].get_span_pos(),
            );
            self.node_ty_map.borrow_mut().insert(
                self.get_node_key(args.get(i).unwrap().id.clone()),
                upgrade_schema_type.clone(),
            );
        }
        // Do keyword argument type check
        for (i, (arg_name, kwarg_ty)) in kwarg_types.iter().enumerate() {
            if !func_ty
                .params
                .iter()
                .map(|p| p.name.clone())
                .any(|x| x == *arg_name)
                && !func_ty.is_variadic
            {
                let (suggs, msg) = self.get_arg_kw_err_suggestion_from_name(arg_name, func_ty);
                self.handler.add_compile_error_with_suggestions(
                    &format!(
                        "{} got an unexpected keyword argument '{}'{}",
                        func_name, arg_name, msg
                    ),
                    kwargs[i].node.arg.get_span_pos(),
                    Some(suggs),
                );
            }
            let expected_types: Vec<TypeRef> = func_ty
                .params
                .iter()
                .filter(|p| p.name == *arg_name)
                .map(|p| p.ty.clone())
                .collect();
            if !expected_types.is_empty() {
                self.must_assignable_to(
                    kwarg_ty.clone(),
                    expected_types[0].clone(),
                    kwargs[i].get_span_pos(),
                    None,
                );
            };
        }
    }

    /// Generate suggestions for keyword argument errors.
    pub(crate) fn get_arg_kw_err_suggestion_from_name(
        &self,
        arg_name: &str,
        func_ty: &FunctionType,
    ) -> (Vec<String>, String) {
        let valid_params: Vec<&str> = func_ty
            .params
            .iter()
            .map(|param| param.name.as_str())
            .collect();
        let suggs = suggestions::provide_suggestions(arg_name, valid_params.into_iter());
        let suggestion = if !suggs.is_empty() {
            format!(", did you mean '{}'?", suggs.join(" or "))
        } else {
            String::new()
        };
        (suggs, suggestion)
    }
}
