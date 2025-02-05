use std::collections::HashMap;

use cairo_vm::{
    hint_processor::{
        builtin_hint_processor::{
            dict_hint_utils::DICT_ACCESS_SIZE,
            dict_manager::{DictKey, DictTracker},
            hint_utils::{
                get_integer_from_var_name, get_maybe_relocatable_from_var_name,
                get_ptr_from_var_name, insert_value_from_var_name,
            },
        },
        hint_processor_definition::HintReference,
    },
    serde::deserialize_program::ApTracking,
    types::{
        errors::math_errors::MathError, exec_scope::ExecutionScopes, relocatable::MaybeRelocatable,
    },
    vm::{
        errors::{hint_errors::HintError, memory_errors::MemoryError},
        vm_core::VirtualMachine,
    },
    Felt252,
};
use revm::precompile::Precompiles;
use starknet_crypto::poseidon_hash_many;

use crate::vm::hints::Hint;

pub const HINTS: &[fn() -> Hint] = &[
    hashdict_read,
    hashdict_write,
    hashdict_read_from_key,
    get_preimage_for_key,
    copy_hashdict_tracker_entry,
    get_keys_for_address_prefix,
    track_precompiles,
];

pub fn hashdict_read() -> Hint {
    Hint::new(
        String::from("hashdict_read"),
        |vm: &mut VirtualMachine,
         exec_scopes: &mut ExecutionScopes,
         ids_data: &HashMap<String, HintReference>,
         ap_tracking: &ApTracking,
         _constants: &HashMap<String, Felt252>|
         -> Result<(), HintError> {
            // Get dictionary pointer and setup tracker
            let dict_ptr = get_ptr_from_var_name("dict_ptr", vm, ids_data, ap_tracking)?;
            let dict_manager_ref = exec_scopes.get_dict_manager()?;
            let mut dict = dict_manager_ref.borrow_mut();
            let tracker = dict.get_tracker_mut(dict_ptr)?;
            tracker.current_ptr.offset += DICT_ACCESS_SIZE;

            let key = get_ptr_from_var_name("key", vm, ids_data, ap_tracking)?;
            let key_len_felt: Felt252 =
                get_integer_from_var_name("key_len", vm, ids_data, ap_tracking)?;
            let key_len: usize = key_len_felt
                .try_into()
                .map_err(|_| MathError::Felt252ToUsizeConversion(Box::new(key_len_felt)))?;

            // Build and process compound key
            let dict_key = build_compound_key(vm, &key, key_len)?;

            tracker.get_value(&dict_key).and_then(|value| {
                insert_value_from_var_name("value", value.clone(), vm, ids_data, ap_tracking)
            })
        },
    )
}

pub fn hashdict_write() -> Hint {
    Hint::new(
        String::from("hashdict_write"),
        |vm: &mut VirtualMachine,
         exec_scopes: &mut ExecutionScopes,
         ids_data: &HashMap<String, HintReference>,
         ap_tracking: &ApTracking,
         _constants: &HashMap<String, Felt252>|
         -> Result<(), HintError> {
            // Get dictionary pointer and setup tracker
            let dict_ptr = get_ptr_from_var_name("dict_ptr", vm, ids_data, ap_tracking)?;
            let dict_manager_ref = exec_scopes.get_dict_manager()?;
            let mut dict = dict_manager_ref.borrow_mut();
            let tracker = dict.get_tracker_mut(dict_ptr)?;
            tracker.current_ptr.offset += DICT_ACCESS_SIZE;

            let key = get_ptr_from_var_name("key", vm, ids_data, ap_tracking)?;
            let key_len_felt: Felt252 =
                get_integer_from_var_name("key_len", vm, ids_data, ap_tracking)?;
            let key_len: usize = key_len_felt
                .try_into()
                .map_err(|_| MathError::Felt252ToUsizeConversion(Box::new(key_len_felt)))?;

            // Build compound key and get new value
            let dict_key = build_compound_key(vm, &key, key_len)?;
            let new_value =
                get_maybe_relocatable_from_var_name("new_value", vm, ids_data, ap_tracking)?;
            let dict_ptr_prev_value = (dict_ptr + 1_i32)?;

            // Update tracker and memory
            tracker.get_value(&dict_key).cloned().and_then(|value| {
                vm.insert_value(dict_ptr_prev_value, value).map_err(|_| {
                    HintError::Memory(MemoryError::UnknownMemoryCell(Box::new(dict_ptr_prev_value)))
                })
            })?;
            tracker.insert_value(&dict_key, &new_value);

            Ok(())
        },
    )
}

pub fn get_keys_for_address_prefix() -> Hint {
    Hint::new(
        String::from("get_keys_for_address_prefix"),
        |vm: &mut VirtualMachine,
         exec_scopes: &mut ExecutionScopes,
         ids_data: &HashMap<String, HintReference>,
         ap_tracking: &ApTracking,
         _constants: &HashMap<String, Felt252>|
         -> Result<(), HintError> {
            // Get dictionary tracker
            let dict_ptr = get_ptr_from_var_name("dict_ptr", vm, ids_data, ap_tracking)?;
            let dict_manager_ref = exec_scopes.get_dict_manager()?;
            let dict = dict_manager_ref.borrow();
            let tracker = dict.get_tracker(dict_ptr)?;

            // Build prefix from memory
            let prefix_ptr = get_ptr_from_var_name("prefix", vm, ids_data, ap_tracking)?;
            let prefix_len_felt: Felt252 =
                get_integer_from_var_name("prefix_len", vm, ids_data, ap_tracking)?;
            let prefix_len: usize = prefix_len_felt
                .try_into()
                .map_err(|_| MathError::Felt252ToUsizeConversion(Box::new(prefix_len_felt)))?;

            let prefix: Vec<MaybeRelocatable> = (0..prefix_len)
                .map(|i| {
                    let addr = (prefix_ptr + i)?;
                    vm.get_maybe(&addr).ok_or_else(|| {
                        HintError::Memory(MemoryError::UnknownMemoryCell(Box::new(addr)))
                    })
                })
                .collect::<Result<_, _>>()?;

            // Find matching preimages
            let matching_preimages: Vec<&Vec<MaybeRelocatable>> = tracker
                .get_dictionary_ref()
                .keys()
                .filter_map(|key| {
                    if let DictKey::Compound(values) = key {
                        // Check if values starts with prefix
                        if values.len() >= prefix.len() && values[..prefix.len()] == prefix[..] {
                            Some(values)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                })
                .collect();

            // Allocate memory segments and write results
            let base = vm.add_memory_segment();
            for (i, preimage) in matching_preimages.iter().enumerate() {
                let ptr = vm.add_memory_segment();
                let bytes32_base = vm.add_memory_segment();

                // Write the rest of preimage (excluding first element) to bytes32_base
                for (j, value) in preimage[1..].iter().enumerate() {
                    vm.insert_value((bytes32_base + j)?, value.clone())?;
                }

                // Write [first_element, bytes32_base] to ptr
                vm.insert_value(ptr, preimage[0].clone())?;
                vm.insert_value((ptr + 1)?, MaybeRelocatable::from(bytes32_base))?;

                // Write ptr to base[i]
                vm.insert_value((base + i)?, MaybeRelocatable::from(ptr))?;
            }

            // Set output values
            insert_value_from_var_name(
                "keys_len",
                Felt252::from(matching_preimages.len()),
                vm,
                ids_data,
                ap_tracking,
            )?;
            insert_value_from_var_name(
                "keys",
                MaybeRelocatable::from(base),
                vm,
                ids_data,
                ap_tracking,
            )?;

            Ok(())
        },
    )
}

pub fn hashdict_read_from_key() -> Hint {
    Hint::new(
        String::from("hashdict_read_from_key"),
        |vm: &mut VirtualMachine,
         exec_scopes: &mut ExecutionScopes,
         ids_data: &HashMap<String, HintReference>,
         ap_tracking: &ApTracking,
         _constants: &HashMap<String, Felt252>|
         -> Result<(), HintError> {
            // Get the hashed key value
            let hashed_key = get_integer_from_var_name("key", vm, ids_data, ap_tracking)?;

            // Get dictionary tracker
            let dict_ptr = get_ptr_from_var_name("dict_ptr_stop", vm, ids_data, ap_tracking)?;
            let dict_manager_ref = exec_scopes.get_dict_manager()?;
            let mut dict = dict_manager_ref.borrow_mut();
            let tracker = dict.get_tracker_mut(dict_ptr)?;

            // Find matching preimage and get its value. This hint can also be called on non-hashed
            // keys.
            let simple_key = DictKey::Simple(hashed_key.into());
            let preimage =
                _get_preimage_for_hashed_key(hashed_key, tracker).unwrap_or(&simple_key).clone();
            let value = tracker
                .get_value(&preimage)
                .map_err(|_| {
                    HintError::CustomHint(
                        format!("No value found for preimage {}", preimage).into(),
                    )
                })?
                .clone();

            // Set the value
            insert_value_from_var_name("value", value, vm, ids_data, ap_tracking)
        },
    )
}

pub fn get_preimage_for_key() -> Hint {
    Hint::new(
        String::from("get_preimage_for_key"),
        |vm: &mut VirtualMachine,
         exec_scopes: &mut ExecutionScopes,
         ids_data: &HashMap<String, HintReference>,
         ap_tracking: &ApTracking,
         _constants: &HashMap<String, Felt252>|
         -> Result<(), HintError> {
            // Get the hashed key value
            let hashed_key = get_integer_from_var_name("key", vm, ids_data, ap_tracking)?;

            // Get dictionary tracker
            let dict_ptr = get_ptr_from_var_name("dict_ptr_stop", vm, ids_data, ap_tracking)?;
            let dict_manager_ref = exec_scopes.get_dict_manager()?;
            let dict = dict_manager_ref.borrow();
            let tracker = dict.get_tracker(dict_ptr)?;

            // Find matching preimage
            let preimage = _get_preimage_for_hashed_key(hashed_key, tracker)?;

            // Write preimage data to memory
            let preimage_data_ptr =
                get_ptr_from_var_name("preimage_data", vm, ids_data, ap_tracking)?;
            if let DictKey::Compound(values) = preimage {
                for (i, value) in values.iter().enumerate() {
                    vm.insert_value((preimage_data_ptr + i)?, value.clone())?;
                }

                // Set preimage length
                insert_value_from_var_name(
                    "preimage_len",
                    Felt252::from(values.len()),
                    vm,
                    ids_data,
                    ap_tracking,
                )?;
            }

            Ok(())
        },
    )
}

pub fn copy_hashdict_tracker_entry() -> Hint {
    Hint::new(
        String::from("copy_hashdict_tracker_entry"),
        |vm: &mut VirtualMachine,
         exec_scopes: &mut ExecutionScopes,
         ids_data: &HashMap<String, HintReference>,
         ap_tracking: &ApTracking,
         _constants: &HashMap<String, Felt252>|
         -> Result<(), HintError> {
            let source_ptr_stop =
                get_ptr_from_var_name("source_ptr_stop", vm, ids_data, ap_tracking)?;
            let dest_ptr = get_ptr_from_var_name("dest_ptr", vm, ids_data, ap_tracking)?;
            let dict_manager_ref = exec_scopes.get_dict_manager()?;
            let mut dict = dict_manager_ref.borrow_mut();

            let source_tracker = dict.get_tracker_mut(source_ptr_stop)?;

            // Find matching preimage from source tracker data
            let key_hash = get_integer_from_var_name("source_key", vm, ids_data, ap_tracking)?;
            let preimage = _get_preimage_for_hashed_key(key_hash, source_tracker)?.clone();
            let value = source_tracker
                .get_value(&preimage)
                .map_err(|_| {
                    HintError::CustomHint(
                        format!("No value found for preimage {}", preimage).into(),
                    )
                })?
                .clone();

            // Update destination tracker
            let dest_tracker = dict.get_tracker_mut(dest_ptr)?;
            dest_tracker.current_ptr.offset += DICT_ACCESS_SIZE;
            dest_tracker.insert_value(&preimage, &value.clone());

            Ok(())
        },
    )
}

pub fn track_precompiles() -> Hint {
    Hint::new(
        String::from("track_precompiles"),
        |vm: &mut VirtualMachine,
         exec_scopes: &mut ExecutionScopes,
         ids_data: &HashMap<String, HintReference>,
         ap_tracking: &ApTracking,
         _constants: &HashMap<String, Felt252>|
         -> Result<(), HintError> {
            // Get dictionary pointer and setup tracker
            let dict_ptr = get_ptr_from_var_name("dict_ptr", vm, ids_data, ap_tracking)?;
            let dict_manager_ref = exec_scopes.get_dict_manager()?;
            let mut dict = dict_manager_ref.borrow_mut();
            let tracker = dict.get_tracker_mut(dict_ptr)?;

            let precompiles = Precompiles::cancun().addresses().collect::<Vec<_>>();
            for address in &precompiles {
                let preimage =
                    vec![MaybeRelocatable::Int(Felt252::from_bytes_le_slice(&address.0 .0))];
                tracker
                    .insert_value(&DictKey::Compound(preimage), &MaybeRelocatable::Int(1.into()));
            }

            tracker.current_ptr.offset += precompiles.len() * DICT_ACCESS_SIZE;

            Ok(())
        },
    )
}

fn build_compound_key(
    vm: &VirtualMachine,
    key: &cairo_vm::types::relocatable::Relocatable,
    key_len: usize,
) -> Result<DictKey, HintError> {
    (0..key_len)
        .map(|i| {
            let mem_addr = (*key + i)?;
            vm.get_maybe(&mem_addr).ok_or_else(|| {
                HintError::Memory(MemoryError::UnknownMemoryCell(Box::from(mem_addr)))
            })
        })
        .collect::<Result<Vec<_>, _>>()
        .map(DictKey::Compound)
}

/// Helper function to find a preimage in a tracker's dictionary given a hashed key
fn _get_preimage_for_hashed_key(
    hashed_key: Felt252,
    tracker: &DictTracker,
) -> Result<&DictKey, HintError> {
    tracker
        .get_dictionary_ref()
        .keys()
        .find(|key| match key {
            DictKey::Compound(values) => {
                let felt_values: Vec<Felt252> = values.iter().filter_map(|v| v.get_int()).collect();
                if felt_values.len() == 1 {
                    felt_values[0] == hashed_key
                } else {
                    poseidon_hash_many(felt_values.iter()) == hashed_key
                }
            }
            _ => false,
        })
        .ok_or_else(|| {
            HintError::CustomHint(format!("No preimage found for hashed key {}", hashed_key).into())
        })
}
