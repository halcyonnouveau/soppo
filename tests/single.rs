mod common;

use common::compile_soppo_file;

mod pass {
    use super::*;

    macro_rules! single_pass_tests {
        ($($name:ident),* $(,)?) => {
            $(
                #[test]
                fn $name() {
                    let fixture = format!("tests/fixtures/single/pass/{}.sop", stringify!($name));
                    let output = compile_soppo_file(&fixture)
                        .expect(&format!("Single-file test '{}' should succeed", stringify!($name)));
                    insta::assert_snapshot!(stringify!($name), output);
                }
            )*
        };
    }

    single_pass_tests!(
        anon_struct,
        basic_go,
        builtins,
        comma_ok,
        compound_nil_check,
        const_grouped_block,
        cstyle_for,
        enum_match,
        enum_variant_methods,
        error_type,
        func_reference,
        generics,
        generic_functions,
        generic_unit_variant,
        goroutines,
        go_interface,
        go_methods,
        go_variables,
        if_init,
        interface_satisfaction,
        iota,
        local_type,
        named_args,
        nil_safety,
        nullable_types,
        rune_literals,
        short_params,
        simple_add,
        string_interpolation,
        struct_match,
        struct_tags,
        try_operator,
        type_alias,
    );
}

mod fail {
    use super::*;

    macro_rules! single_fail_tests {
        ($($name:ident),* $(,)?) => {
            $(
                #[test]
                fn $name() {
                    let fixture = format!("tests/fixtures/single/fail/{}.sop", stringify!($name));
                    let result = compile_soppo_file(&fixture);
                    assert!(result.is_err(), "Test '{}' should fail", stringify!($name));
                    insta::assert_snapshot!(stringify!($name), result.unwrap_err());
                }
            )*
        };
    }

    single_fail_tests!(
        assign_wrong_type,
        const_no_value,
        duration_string_mul,
        generic_constraint_comparable,
        generic_unit_variant_no_type_args,
        go_unknown_function,
        go_wrong_arg_count,
        go_wrong_arg_type,
        interface_not_satisfied,
        named_arg_duplicate,
        named_arg_missing,
        named_arg_unknown,
        named_arg_wrong_order,
        nil_access_wrong_branch,
        nil_chan_send,
        nil_deref_no_check,
        nil_func_arg,
        nil_map_value,
        nil_nested_no_check,
        nil_reassign_resets,
        nil_return_to_nonnull,
        nil_slice_element,
        nil_struct_field,
        non_exhaustive,
        nullable_assign_to_nonnull,
        nullable_nil_to_nonnull,
        nullable_no_init,
        try_expr_no_error,
        try_no_return_error,
        type_mismatch,
        undeclared_variable,
        var_no_type_or_value,
    );
}
