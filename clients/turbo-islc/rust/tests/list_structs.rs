#[path = "./util.rs"]
mod util;

use std::sync::Arc;

use parking_lot::RwLock;

use turbo_isl::turbo;
use util::WasmString;

turbo!("./tests/list-structs.tisl" -> wasmtime);

#[derive(Clone, Default)]
struct School {
    fishes: Arc<RwLock<Vec<ocean::Fish>>>,
}

impl ocean::OceanApi for School {
    fn add_fish_to_school(&mut self, fish: &ocean::Fish) -> Result<(), String> {
        self.fishes.write().push(fish.clone());
        Ok(())
    }

    fn add_fishes_to_school(&mut self, fishes: &[ocean::Fish]) -> Result<(), String> {
        self.fishes.write().extend_from_slice(fishes);
        Ok(())
    }

    fn compare(&mut self, fish1: &ocean::Fish, fish2: &ocean::Fish) -> Result<ocean::Fish, String> {
        Ok(if fish1.size > fish2.size {
            fish1
        } else {
            fish2
        }
        .clone())
    }

    fn get_fish(&mut self, name: &str) -> Result<&ocean::Fish, String> {
        self.fishes
            .read()
            .iter()
            .find(|f| f.name == name)
            .ok_or_else(|| String::from("Could not find fish"))
    }

    fn get_fishes_of_type(
        &mut self,
        fish_type: ocean::FishType,
    ) -> Result<Vec<ocean::Fish>, String> {
        Ok(self
            .fishes
            .read()
            .iter()
            .filter(|fesh| fish_type == fesh.r#type)
            .cloned()
            .collect())
    }
}

#[test]
fn test_list_structs() {
    // Initialzing test environment
    let mut context = util::WasmTestContext::new(5, School::default());

    unsafe {
        context.setup_mock_functions(&mut ocean::GET_FUNCTION, &mut ocean::GET_MEMORY);
    }

    assert!(
        ocean::add_to_linker(&mut context.linker).is_ok(),
        "Expected to be able to add ocean module to linker."
    );

    // Grab the function we want to call
    let func = context.get_function("ocean", "get_fish");

    assert!(
        func.is_some(),
        "Expected fish to be a function inside the ocean module."
    );
    let func = func.unwrap();

    // Prepare input and output arguments
    let fish_name = WasmString::from_str(context.mem_base, &context.allocator, "berggylta");
    let big_result = context.allocator.allocate(1);
    let name_result = context.allocator.allocate_ptr();

    // Small fish
    let result = context.call_function(
        func,
        &[
            // inputs
            fish_name.into(),
            1.0f64.into(),
            (false as i32).into(),
            // outputs
            big_result.into(),
            name_result.into(),
        ],
    );
    assert!(result.is_ok());
    let big = *big_result.to_host::<u8>(context.mem_base) == 1;
    let name = WasmString::new_indirect(name_result, context.mem_base);

    assert_eq!(name.to_str(), Ok("berggylta"));
    assert!(
        !big,
        "Expected a berggylta of size 1 to be considered small."
    );
}
