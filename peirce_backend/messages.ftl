peirce_backend_consider_type_length_limit =
    consider adding a `#![type_length_limit="{$type_length}"]` attribute to your crate

peirce_backend_couldnt_dump_mono_stats =
    unexpected error occurred while dumping monomorphization stats: {$error}

peirce_backend_encountered_error_while_instantiating =
    the above error was encountered while instantiating `{$formatted_item}`

peirce_backend_large_assignments =
    moving {$size} bytes
    .label = value moved from here
    .note = The current maximum size is {$limit}, but it can be customized with the move_size_limit attribute: `#![move_size_limit = "..."]`

peirce_backend_no_optimized_mir =
    missing optimized MIR for an item in the crate `{$crate_name}`
    .note = missing optimized MIR for this item (was the crate `{$crate_name}` compiled with `--emit=metadata`?)

peirce_backend_recursion_limit =
    reached the recursion limit while instantiating `{$shrunk}`
    .note = `{$def_path_str}` defined here

peirce_backend_symbol_already_defined = symbol `{$symbol}` is already defined

peirce_backend_type_length_limit = reached the type-length limit while instantiating `{$shrunk}`

peirce_backend_unknown_cgu_collection_mode =
    unknown codegen-item collection mode '{$mode}', falling back to 'lazy' mode

peirce_backend_unused_generic_params = item has unused generic parameters

peirce_backend_written_to_path = the full type name has been written to '{$path}'
