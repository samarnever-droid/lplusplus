use crate::mir::ir::*;
use crate::escape::StorageClass;
use crate::semantic::BindingId;
use std::collections::HashMap;

/// MIR Transformation Pass: ARC Insertion
///
/// Looks up each Local's BindingId in the escape_map. If it maps to StorageClass::Arc,
/// inserts Retain and Release instructions appropriately.
pub fn run_arc_insertion_pass(program: &mut MirProgram, escape_map: &HashMap<BindingId, StorageClass>) {
    for (_, func) in program.functions.iter_mut() {
        // Find which locals are Arc
        let mut arc_locals = Vec::new();
        for local in &func.locals {
            if let Some(b_id) = local.binding_id {
                if let Some(&StorageClass::Arc) = escape_map.get(&b_id) {
                    arc_locals.push(local.id);
                }
            }
        }
        
        if arc_locals.is_empty() {
            continue;
        }
        
        // Insert Retain after assignment, Release before Return.
        for block in &mut func.blocks {
            let mut i = 0;
            while i < block.instrs.len() {
                if let MirInstr::Assign(lhs, _) = &block.instrs[i] {
                    if arc_locals.contains(lhs) {
                        block.instrs.insert(i + 1, MirInstr::Retain(*lhs));
                        i += 1;
                    }
                }
                i += 1;
            }
            
            if let Terminator::Return(_) = &block.terminator {
                for &l_id in &arc_locals {
                    block.instrs.push(MirInstr::Release(l_id));
                }
            }
        }
    }
}
