#[path = "./util.rs"]
mod util;

use std::collections::HashMap;

use turbo_isl::turbo;

use util::WasmString;

turbo!("./tests/minimal.tisl" -> wasmtime);

#[derive(Clone)]
struct Pacific {
    fishes: HashMap<&'static str, f64>,
}

impl Default for Pacific {
    fn default() -> Self {
        let mut fishes = HashMap::new();
        fishes.insert("berggylta", 2f64);
        fishes.insert("bergtunga", 1f64);
        fishes.insert("fjärsing", 0.3);
        fishes.insert("knot", 10f64);
        fishes.insert("lubb", 15f64);
        Self { fishes }
    }
}

impl ocean::OceanApi for Pacific {
    fn fish(&mut self, name: &str, size: f64, is_angry: bool) -> Result<ocean::FishResult, String> {
        if let Some(normal_size) = self.fishes.get(name) {
            Ok(ocean::FishResult {
                big: size > *normal_size || is_angry,
                name: name.to_owned(),
            })
        } else {
            Err(String::from("Could not find the fish."))
        }
    }
}

#[test]
fn ocean() {
    // Initialzing test environment
    let mut context = util::WasmTestContext::new(5, Pacific::default());

    unsafe {
        context.setup_mock_functions(&mut ocean::GET_FUNCTION, &mut ocean::GET_MEMORY);
    }

    assert!(
        ocean::add_to_linker(&mut context.linker).is_ok(),
        "Expected to be able to add ocean module to linker."
    );

    // Grab the function we want to call
    let func = context.get_function("ocean", "fish");

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

    // Big fish
    let fish_name = *WasmString::from_str(context.mem_base, &context.allocator, "lubb");
    let big_result = context.allocator.allocate(1);
    let name_result = context.allocator.allocate_ptr();

    let result = context.call_function(
        func,
        &[
            // inputs
            fish_name.into(),
            20f64.into(),
            (false as i32).into(),
            // outputs
            big_result.into(),
            name_result.into(),
        ],
    );
    assert!(result.is_ok());
    let big = *big_result.to_host::<u8>(context.mem_base) == 1;
    let name = WasmString::new_indirect(name_result, context.mem_base);

    assert_eq!(name.to_str(), Ok("lubb"));
    assert!(big, "Expected a lubb of size 20 to be considered big.");

    // Angry fish 🐡
    let fish_name = *WasmString::from_str(context.mem_base, &context.allocator, "lubb");
    let big_result = context.allocator.allocate(1);
    let name_result = context.allocator.allocate_ptr();

    let result = context.call_function(
        func,
        &[
            // inputs
            fish_name.into(),
            2f64.into(),
            (true as i32).into(),
            // outputs
            big_result.into(),
            name_result.into(),
        ],
    );
    assert!(result.is_ok());
    let big = *big_result.to_host::<u8>(context.mem_base) == 1;
    let name = WasmString::new_indirect(name_result, context.mem_base);

    assert_eq!(name.to_str(), Ok("lubb"));
    assert!(
        big,
        "Expected a lubb of size 2 to be considered big since it is (very?) angry!"
    );

    // Error fish
    let fish_name = *WasmString::from_str(context.mem_base, &context.allocator, "kubb");
    let result = context.call_function(
        func,
        &[
            fish_name.into(),
            20f64.into(),
            (false as i32).into(),
            big_result.into(),
            name_result.into(),
        ],
    );
    assert!(result.is_err(), "Did not expect to find fish \"kubb\"");
}
