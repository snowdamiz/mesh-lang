use inkwell::attributes::{Attribute, AttributeLoc};
use inkwell::builder::BuilderError;
use inkwell::module::Linkage;
use inkwell::values::FunctionValue;
use inkwell::{AddressSpace, IntPredicate};

use super::CodeGen;

impl<'ctx> CodeGen<'ctx> {
    pub(super) fn inline_list_reads(&self) -> Result<(), BuilderError> {
        let (Some(length), Some(get)) = (
            self.module.get_function("mesh_list_length"),
            self.module.get_function("mesh_list_get"),
        ) else {
            return Ok(());
        };
        let word = self.context.i64_type();
        let ptr = self.context.ptr_type(AddressSpace::default());
        let builder = self.context.create_builder();
        let length = self.inline_list_intrinsic(length, "mesh_list_length.inline");
        builder.position_at_end(self.context.append_basic_block(length, "entry"));
        let list = length.get_nth_param(0).unwrap().into_pointer_value();
        let len = builder.build_load(word, list, "length")?;
        builder.build_return(Some(&len))?;

        let fallback = get;
        let get = self.inline_list_intrinsic(get, "mesh_list_get.inline");
        let entry = self.context.append_basic_block(get, "entry");
        let bounds_error = self.context.append_basic_block(get, "bounds_error");
        let select_data = self.context.append_basic_block(get, "select_data");
        let view = self.context.append_basic_block(get, "view");
        let read = self.context.append_basic_block(get, "read");
        builder.position_at_end(entry);
        let list = get.get_nth_param(0).unwrap().into_pointer_value();
        let index = get.get_nth_param(1).unwrap().into_int_value();
        let len = builder.build_load(word, list, "length")?.into_int_value();
        let in_bounds = builder.build_int_compare(IntPredicate::ULT, index, len, "in_bounds")?;
        builder.build_conditional_branch(in_bounds, select_data, bounds_error)?;

        builder.position_at_end(bounds_error);
        builder.build_call(fallback, &[list.into(), index.into()], "bounds_panic")?;
        builder.build_unreachable()?;

        // The layout mirrors mesh-rt's collections::list::list_data. An owned
        // list stores slots after its two header words. A view stores an owned
        // parent and an offset in slots after the same header.
        builder.position_at_end(select_data);
        let slot_at = |base, offset| unsafe {
            builder.build_gep(word, base, &[word.const_int(offset, false)], "slot")
        };
        let capacity = builder
            .build_load(word, slot_at(list, 1)?, "capacity")?
            .into_int_value();
        let owned_data = slot_at(list, 2)?;
        let is_view = builder.build_int_compare(
            IntPredicate::EQ,
            capacity,
            word.const_all_ones(),
            "is_view",
        )?;
        builder.build_conditional_branch(is_view, view, read)?;

        builder.position_at_end(view);
        let parent = builder
            .build_load(word, owned_data, "parent_word")?
            .into_int_value();
        let parent = builder.build_int_to_ptr(parent, ptr, "parent")?;
        let offset = builder
            .build_load(word, slot_at(list, 3)?, "offset")?
            .into_int_value();
        let parent_data = slot_at(parent, 2)?;
        let view_data = unsafe { builder.build_gep(word, parent_data, &[offset], "view_data")? };
        builder.build_unconditional_branch(read)?;

        builder.position_at_end(read);
        let data = builder.build_phi(ptr, "data")?;
        data.add_incoming(&[(&owned_data, select_data), (&view_data, view)]);
        let data = data.as_basic_value().into_pointer_value();
        let slot = unsafe { builder.build_gep(word, data, &[index], "element")? };
        let element = builder.build_load(word, slot, "value")?;
        builder.build_return(Some(&element))?;
        Ok(())
    }

    fn inline_list_intrinsic(
        &self,
        intrinsic: FunctionValue<'ctx>,
        name: &str,
    ) -> FunctionValue<'ctx> {
        let inline = self
            .module
            .add_function(name, intrinsic.get_type(), Some(Linkage::Private));
        inline.add_attribute(
            AttributeLoc::Function,
            self.context
                .create_enum_attribute(Attribute::get_named_enum_kind_id("alwaysinline"), 0),
        );
        intrinsic.replace_all_uses_with(inline);
        inline
    }
}
