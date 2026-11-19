use inkwell::attributes::{Attribute, AttributeLoc};
use inkwell::module::Linkage;
use inkwell::passes::PassBuilderOptions;
use inkwell::values::{CallSiteValue, FunctionValue, InstructionOpcode};

use super::{CodeGen, BUILT};

fn calls(function: FunctionValue<'_>) -> impl Iterator<Item = CallSiteValue<'_>> {
    function.get_basic_block_iter().flat_map(|block| {
        block
            .get_instructions()
            .filter_map(|ins| CallSiteValue::try_from(ins).ok())
    })
}

impl CodeGen<'_> {
    pub(super) fn lower_supervisor_failures(&self) -> Result<(), String> {
        let panic = self.module.get_function("mesh_panic_str").unwrap();
        let fail = self.module.get_function("mesh_actor_fail").unwrap();
        let inline = self
            .context
            .create_enum_attribute(Attribute::get_named_enum_kind_id("alwaysinline"), 0);
        let mut entries = Vec::new();
        let builder = self.context.create_builder();
        for &(entry, store) in &self.supervisor_entries {
            let mut has_panic = false;
            for call in calls(entry) {
                let Some(body) = call.get_called_fn_value() else {
                    continue;
                };
                if body == panic {
                    has_panic = true;
                } else if calls(body).any(|c| c.get_called_fn_value() == Some(panic)) {
                    call.add_attribute(AttributeLoc::Function, inline);
                    has_panic = true;
                }
            }
            if !has_panic {
                continue;
            }

            // Only this private entry can return directly to the scheduler.
            // The same actor body or start function called elsewhere must unwind.
            let root = self.module.add_function(
                "supervised.start",
                entry.get_type(),
                Some(Linkage::Private),
            );
            builder.position_at_end(self.context.append_basic_block(root, "entry"));
            let call = builder
                .build_call(entry, &[root.get_first_param().unwrap().into()], "")
                .expect(BUILT);
            call.add_attribute(AttributeLoc::Function, inline);
            builder
                .build_return(Some(&self.context.struct_type(&[], false).const_zero()))
                .expect(BUILT);
            let address = root
                .as_global_value()
                .as_pointer_value()
                .const_to_int(self.context.i64_type());
            assert!(store.set_operand(0, address));
            entries.push(root);
        }
        if entries.is_empty() {
            return Ok(());
        }
        self.module
            .run_passes(
                "always-inline,function(simplifycfg)",
                &self.target_machine,
                PassBuilderOptions::create(),
            )
            .map_err(|e| e.to_string())?;
        for entry in entries {
            for block in entry.get_basic_block_iter() {
                let Some(end) = block.get_terminator() else {
                    continue;
                };
                if end.get_opcode() != InstructionOpcode::Unreachable {
                    continue;
                }
                let Some(ins) = end.get_previous_instruction() else {
                    continue;
                };
                if CallSiteValue::try_from(ins)
                    .ok()
                    .and_then(CallSiteValue::get_called_fn_value)
                    != Some(panic)
                {
                    continue;
                }
                let message = ins.get_operand(0).unwrap().value().unwrap();
                builder.position_before(&ins);
                builder
                    .build_call(fail, &[message.into()], "")
                    .expect(BUILT);
                builder
                    .build_return(Some(&self.context.struct_type(&[], false).const_zero()))
                    .expect(BUILT);
                end.erase_from_basic_block();
                ins.erase_from_basic_block();
            }
        }
        self.verify()
    }
}
