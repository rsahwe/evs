#![warn(clippy::all, clippy::pedantic, clippy::cargo)]
#![allow(clippy::uninlined_format_args, reason = "Why is this even a thing?")]
#![allow(
    clippy::unnecessary_debug_formatting,
    reason = "I deliberately choose between display and debug."
)]
#![allow(clippy::if_not_else, reason = "Style choice.")]
//TODO: DOCUMENTATION
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    missing_docs,
    clippy::missing_docs_in_private_items,
    clippy::undocumented_unsafe_blocks,
    reason = "Documentation will be done later."
)]
#![warn(
    clippy::absolute_paths,
    clippy::allow_attributes_without_reason,
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::assertions_on_result_states,
    clippy::big_endian_bytes,
    clippy::cfg_not_test,
    clippy::clone_on_ref_ptr,
    clippy::create_dir,
    clippy::dbg_macro,
    clippy::default_numeric_fallback,
    clippy::default_union_representation,
    clippy::deref_by_slicing,
    clippy::disallowed_script_idents,
    clippy::doc_include_without_cfg,
    clippy::doc_paragraphs_missing_punctuation,
    clippy::else_if_without_else,
    clippy::empty_drop,
    clippy::empty_enum_variants_with_brackets,
    clippy::empty_structs_with_brackets,
    clippy::error_impl_error,
    clippy::exit,
    clippy::expect_used,
    clippy::field_scoped_visibility_modifiers,
    clippy::filetype_is_file,
    clippy::float_arithmetic,
    clippy::get_unwrap,
    clippy::host_endian_bytes,
    clippy::if_then_some_else_none,
    clippy::impl_trait_in_params,
    clippy::indexing_slicing,
    clippy::infinite_loop,
    clippy::inline_asm_x86_att_syntax,
    clippy::lossy_float_literal,
    clippy::map_err_ignore,
    clippy::map_with_unused_argument_over_ranges,
    clippy::mem_forget,
    clippy::missing_assert_message,
    clippy::missing_inline_in_public_items,
    clippy::mixed_read_write_in_expression,
    clippy::mod_module_files,
    clippy::multiple_inherent_impl,
    clippy::multiple_unsafe_ops_per_block,
    clippy::mutex_atomic,
    clippy::panic,
    clippy::partial_pub_fields,
    clippy::pathbuf_init_then_push,
    clippy::pointer_format,
    clippy::precedence_bits,
    clippy::pub_use,
    clippy::rc_buffer,
    clippy::rc_mutex,
    clippy::redundant_test_prefix,
    clippy::redundant_type_annotations,
    clippy::renamed_function_params,
    clippy::rest_pat_in_fully_bound_structs,
    clippy::return_and_then,
    clippy::same_name_method,
    clippy::semicolon_outside_block,
    clippy::separated_literal_suffix,
    clippy::str_to_string,
    clippy::string_lit_chars_any,
    clippy::string_slice,
    clippy::suspicious_xor_used_as_pow,
    clippy::tests_outside_test_module,
    clippy::todo,
    clippy::try_err,
    clippy::unimplemented,
    clippy::unnecessary_safety_comment,
    clippy::unnecessary_safety_doc,
    clippy::unnecessary_self_imports,
    clippy::unneeded_field_pattern,
    clippy::unused_result_ok,
    clippy::unused_trait_names,
    clippy::verbose_file_reads
)]

#[cfg(target_pointer_width = "32")]
compile_error!("32 bit is not supported.");

pub mod cli;
pub mod diff;
pub mod error;
pub mod objects;
pub mod repo;
pub mod store;
pub mod util;
