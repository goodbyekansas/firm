pub mod ocean {
    macro_rules! try_or_errmsg {
        ($caller:ident, $expression:expr) => {
            match $expression {
                Ok(value) => value,
                Err(e) => {
                    return create_wasm_string(&mut $caller, &format!("Error: {}", e))
                        .map(|s| s as i64)
                        .unwrap_or(-1i64);
                }
            }
        };
    }

    pub trait OceanApi {
        fn add_fish_to_school<'input>(&mut self, fish: &'input Fish) -> Result<(), String>;

        fn add_fishes_to_school<'input>(&mut self, fishes: &'input [Fish]) -> Result<(), String>;

        fn compare<'input>(
            &mut self,
            fish1: &'input Fish,
            fish2: &'input Fish,
        ) -> Result<Fish, String>;

        fn get_fish<'input>(&mut self, name: &'input str) -> Result<&Fish, String>;

        fn get_fishes_of_type<'input>(&mut self, r#type: FishType) -> Result<Vec<Fish>, String>;
    }

    #[repr(u8)]
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub enum FishType {
        Bony = 0,
        Jawless = 1,
        Cartilaginous = 2,
    }

    impl TryFrom<u8> for FishType {
        type Error = String;
        fn try_from(value: u8) -> Result<Self, Self::Error> {
            match value {
                0 => Ok(FishType::Bony),
                1 => Ok(FishType::Jawless),
                2 => Ok(FishType::Cartilaginous),
                v => Err(format!(r#"Enum "FishType" of {} out of bounds."#, v)),
            }
        }
    }

    #[derive(Clone, Debug)]
    pub struct Fish {
        pub name: String,
        pub size: f64,
        pub r#type: FishType,
        pub data: Vec<u8>,
    }

    pub fn add_to_linker<T: OceanApi + Clone + Send + Sync + 'static>(
        linker: &mut wasmtime::Linker<T>,
    ) -> Result<(), String> {
        linker
            .func_wrap(
                "ocean",
                "__add_fish_to_school",
                wrappers::add_fish_to_school,
            )
            .map_err(|e| e.to_string())?;
        linker
            .func_wrap(
                "ocean",
                "__add_fishes_to_school",
                wrappers::add_fishes_to_school,
            )
            .map_err(|e| e.to_string())?;
        linker
            .func_wrap("ocean", "__compare", wrappers::compare)
            .map_err(|e| e.to_string())?;
        linker
            .func_wrap("ocean", "__get_fish", wrappers::get_fish)
            .map_err(|e| e.to_string())?;
        linker
            .func_wrap(
                "ocean",
                "__get_fishes_of_type",
                wrappers::get_fishes_of_type,
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    #[cfg(test)]
    pub static mut GET_FUNCTION: Option<Box<dyn Fn(&str) -> Option<wasmtime::Func>>> = None;

    #[cfg(test)]
    pub static mut GET_MEMORY: Option<Box<dyn Fn(&str) -> Option<wasmtime::Memory>>> = None;

    #[allow(unused_assignments, unused_variables)]
    mod wrappers {
        use wasmtime::AsContextMut;

        const FISH_SIZE: usize = std::mem::size_of::<i64>()
            + std::mem::size_of::<f64>()
            + 1
            + (std::mem::size_of::<i64>() * 2);

        fn get_string(mem_base: *mut u8, offset: usize) -> Result<&'static str, String> {
            unsafe {
                let host_ptr = mem_base.add(offset as usize);
                std::ffi::CStr::from_ptr(host_ptr as *const i8)
                    .to_str()
                    .map_err(|e| format!("UTF-8 error: {}", e))
            }
        }

        fn get_func<T>(
            caller: &mut wasmtime::Caller<'_, T>,
            func: &str,
        ) -> Result<wasmtime::Func, String> {
            #[cfg(test)]
            {
                (unsafe {
                    super::GET_FUNCTION
                        .as_ref()
                        .expect("Forgot to set the global GET_FUNCTION?")
                }(func))
                .ok_or_else(|| format!("Failed to get function `{}`.", func))
            }

            #[cfg(not(test))]
            {
                caller
                    .get_export(func)
                    .and_then(|e| e.into_func())
                    .ok_or_else(|| format!("Failed to get function `{}`.", func))
            }
        }

        fn get_memory<T>(
            caller: &mut wasmtime::Caller<'_, T>,
            mem: &str,
        ) -> Result<wasmtime::Memory, String> {
            #[cfg(test)]
            {
                (unsafe {
                    super::GET_MEMORY
                        .as_ref()
                        .expect("Forgot to set the global GET_MEMORY?")
                }(mem))
                .ok_or_else(|| format!("Failed to get memory `{}`", mem))
            }

            #[cfg(not(test))]
            {
                caller
                    .get_export(mem)
                    .and_then(|e| e.into_memory())
                    .ok_or_else(|| format!("Failed to get memory `{}`", mem))
            }
        }

        fn allocate<T>(
            caller: &mut wasmtime::Caller<'_, T>,
            amount: usize,
        ) -> Result<usize, String> {
            let allocator = &get_func(caller, "allocate_wasm_mem")?;
            let mut returns = [wasmtime::Val::null()];
            allocator
                .call(caller, &[(amount as i64).into()], &mut returns)
                .map_err(|e| e.to_string())
                .and_then(|_| {
                    returns[0]
                        .i64()
                        .ok_or_else(|| String::from("Allocation function returned the wrong type"))
                })
                .map(|x| x as usize)
        }

        fn create_wasm_string<T>(
            caller: &mut wasmtime::Caller<'_, T>,
            text: &str,
        ) -> Result<usize, String> {
            let offset = allocate(caller, text.len() + 1)?;
            let mem = &get_memory(caller, "memory")?;
            mem.write(caller.as_context_mut(), offset, text.as_bytes())
                .map_err(|e| format!("Failed to write string: {}", e))?;
            mem.write(caller, offset + text.len(), &[b'\0'])
                .map_err(|e| format!("Failed to write null terminator: {}", e))?;
            Ok(offset)
        }

        fn create_wasm_string2<T>(
            caller: &mut wasmtime::Caller<'_, T>,
            ptr: *const u8,
            len: usize,
        ) -> Result<usize, String> {
            let offset = allocate(caller, len + 1)?;
            let mem = &get_memory(caller, "memory")?;
            unsafe {
                mem.data_ptr(caller.as_context_mut())
                    .add(offset)
                    .copy_from_nonoverlapping(ptr, len);
                *mem.data_ptr(caller.as_context_mut()).add(offset + len) = b'\0';
            }
            Ok(offset)
        }

        pub fn add_fish_to_school<T: super::OceanApi>(
            mut caller: wasmtime::Caller<'_, T>,
            fish: i64,
        ) -> i64 {
            let mem = *try_or_errmsg!(caller, &get_memory(&mut caller, "memory"));
            let mem_base = mem.data_ptr(&caller);
            let data = caller.data_mut();

            // Convert input "fish"
            let fish = &{
                let record_base = unsafe { mem_base.add(fish as usize) as *const u8 };

                super::Fish {
                    name: {
                        let value = unsafe { record_base.add(0) };
                        let name = unsafe { *(value as *const i64) };
                        try_or_errmsg!(caller, get_string(mem_base, name as usize)).to_string()
                    },
                    size: {
                        let value = unsafe { record_base.add(std::mem::size_of::<i64>()) };
                        unsafe { *(value as *const f64) }
                    },
                    r#type: {
                        let value = unsafe {
                            record_base.add(std::mem::size_of::<i64>() + std::mem::size_of::<f64>())
                        };
                        let r#type = unsafe { *(value as *const u8) };
                        try_or_errmsg!(caller, super::FishType::try_from(r#type as u8))
                    },
                    data: {
                        let value = unsafe {
                            record_base
                                .add(std::mem::size_of::<i64>() + std::mem::size_of::<f64>() + 1)
                        };
                        let data = unsafe { *(value as *const i64) };
                        let data_len = {
                            let value = unsafe {
                                record_base.add(
                                    std::mem::size_of::<i64>()
                                        + std::mem::size_of::<i64>()
                                        + std::mem::size_of::<f64>()
                                        + 1,
                                )
                            };
                            unsafe { *(value as *const i64) }
                        };
                        {
                            let mut vector: Vec<u8> = Vec::with_capacity(data_len as usize);
                            let array_ptr = unsafe { mem_base.add(data as usize) as *const u8 };
                            let slice =
                                unsafe { std::slice::from_raw_parts(array_ptr, data_len as usize) };

                            slice.iter().for_each(|data| {
                                vector.push(*data);
                            });
                            vector
                        }
                    },
                }
            };

            let native_res = try_or_errmsg!(caller, data.add_fish_to_school(fish));

            0
        }

        pub fn add_fishes_to_school<T: super::OceanApi>(
            mut caller: wasmtime::Caller<'_, T>,
            fishes: i64,
            fishes_len: i64,
        ) -> i64 {
            let mem = *try_or_errmsg!(caller, &get_memory(&mut caller, "memory"));
            let mem_base = mem.data_ptr(&caller);
            let data = caller.data_mut();

            // Convert input "fishes"
            let fishes = &{
                let mut vector: Vec<super::Fish> = Vec::with_capacity(fishes_len as usize);
                let array_offset = fishes as usize;

                for i in 0..(fishes_len as usize) {
                    let fishes = array_offset + i * FISH_SIZE;
                    vector.push({
                        let record_base = unsafe { mem_base.add(fishes as usize) as *const u8 };

                        super::Fish {
                            name: {
                                let value = unsafe { record_base.add(0) };
                                let name = unsafe { *(value as *const i64) };
                                try_or_errmsg!(caller, get_string(mem_base, name as usize))
                                    .to_string()
                            },
                            size: {
                                let value = unsafe { record_base.add(std::mem::size_of::<i64>()) };
                                unsafe { *(value as *const f64) }
                            },
                            r#type: {
                                let value = unsafe {
                                    record_base.add(
                                        std::mem::size_of::<i64>() + std::mem::size_of::<f64>(),
                                    )
                                };
                                let r#type = unsafe { *(value as *const u8) };
                                try_or_errmsg!(caller, super::FishType::try_from(r#type as u8))
                            },
                            data: {
                                let value = unsafe {
                                    record_base.add(
                                        std::mem::size_of::<i64>() + std::mem::size_of::<f64>() + 1,
                                    )
                                };
                                let data = unsafe { *(value as *const i64) };
                                let data_len = {
                                    let value = unsafe {
                                        record_base.add(
                                            std::mem::size_of::<i64>()
                                                + std::mem::size_of::<i64>()
                                                + std::mem::size_of::<f64>()
                                                + 1,
                                        )
                                    };
                                    unsafe { *(value as *const i64) }
                                };
                                {
                                    let mut vector: Vec<u8> = Vec::with_capacity(data_len as usize);
                                    let array_ptr =
                                        unsafe { mem_base.add(data as usize) as *const u8 };
                                    let slice = unsafe {
                                        std::slice::from_raw_parts(array_ptr, data_len as usize)
                                    };

                                    slice.iter().for_each(|data| {
                                        vector.push(*data);
                                    });
                                    vector
                                }
                            },
                        }
                    });
                }
                vector
            };

            let native_res = try_or_errmsg!(caller, data.add_fishes_to_school(fishes));

            0
        }

        pub fn compare<T: super::OceanApi>(
            mut caller: wasmtime::Caller<'_, T>,
            fish1: i64,
            fish2: i64,
            winner_out: i64,
        ) -> i64 {
            let mem = *try_or_errmsg!(caller, &get_memory(&mut caller, "memory"));
            let mem_base = mem.data_ptr(&caller);
            let data = caller.data_mut();

            // Convert input "fish1"
            let fish1 = &{
                let record_base = unsafe { mem_base.add(fish1 as usize) as *const u8 };

                super::Fish {
                    name: {
                        let value = unsafe { record_base.add(0) };
                        let name = unsafe { *(value as *const i64) };
                        try_or_errmsg!(caller, get_string(mem_base, name as usize)).to_string()
                    },
                    size: {
                        let value = unsafe { record_base.add(std::mem::size_of::<i64>()) };
                        unsafe { *(value as *const f64) }
                    },
                    r#type: {
                        let value = unsafe {
                            record_base.add(std::mem::size_of::<i64>() + std::mem::size_of::<f64>())
                        };
                        let r#type = unsafe { *(value as *const u8) };
                        try_or_errmsg!(caller, super::FishType::try_from(r#type as u8))
                    },
                    data: {
                        let value = unsafe {
                            record_base
                                .add(std::mem::size_of::<i64>() + std::mem::size_of::<f64>() + 1)
                        };
                        let data = unsafe { *(value as *const i64) };
                        let data_len = {
                            let value = unsafe {
                                record_base.add(
                                    std::mem::size_of::<i64>()
                                        + std::mem::size_of::<i64>()
                                        + std::mem::size_of::<f64>()
                                        + 1,
                                )
                            };
                            unsafe { *(value as *const i64) }
                        };
                        {
                            let mut vector: Vec<u8> = Vec::with_capacity(data_len as usize);
                            let array_ptr = unsafe { mem_base.add(data as usize) as *const u8 };
                            let slice =
                                unsafe { std::slice::from_raw_parts(array_ptr, data_len as usize) };

                            slice.iter().for_each(|data| {
                                vector.push(*data);
                            });
                            vector
                        }
                    },
                }
            };
            // Convert input "fish2"
            let fish2 = &{
                let record_base = unsafe { mem_base.add(fish2 as usize) as *const u8 };

                super::Fish {
                    name: {
                        let value = unsafe { record_base.add(0) };
                        let name = unsafe { *(value as *const i64) };
                        try_or_errmsg!(caller, get_string(mem_base, name as usize)).to_string()
                    },
                    size: {
                        let value = unsafe { record_base.add(std::mem::size_of::<i64>()) };
                        unsafe { *(value as *const f64) }
                    },
                    r#type: {
                        let value = unsafe {
                            record_base.add(std::mem::size_of::<i64>() + std::mem::size_of::<f64>())
                        };
                        let r#type = unsafe { *(value as *const u8) };
                        try_or_errmsg!(caller, super::FishType::try_from(r#type as u8))
                    },
                    data: {
                        let value = unsafe {
                            record_base
                                .add(std::mem::size_of::<i64>() + std::mem::size_of::<f64>() + 1)
                        };
                        let data = unsafe { *(value as *const i64) };
                        let data_len = {
                            let value = unsafe {
                                record_base.add(
                                    std::mem::size_of::<i64>()
                                        + std::mem::size_of::<i64>()
                                        + std::mem::size_of::<f64>()
                                        + 1,
                                )
                            };
                            unsafe { *(value as *const i64) }
                        };
                        {
                            let mut vector: Vec<u8> = Vec::with_capacity(data_len as usize);
                            let array_ptr = unsafe { mem_base.add(data as usize) as *const u8 };
                            let slice =
                                unsafe { std::slice::from_raw_parts(array_ptr, data_len as usize) };

                            slice.iter().for_each(|data| {
                                vector.push(*data);
                            });
                            vector
                        }
                    },
                }
            };

            let native_res = try_or_errmsg!(caller, data.compare(fish1, fish2));

            // Write output "winner"
            // Write record Winner
            {
                let record_base = unsafe { mem_base.add(winner_out as usize) as *mut u8 };
                // Write field Winner.name
                {
                    let name_out = winner_out as usize;
                    unsafe {
                        *(mem_base.add(name_out as usize) as *mut i64) = try_or_errmsg!(
                            caller,
                            create_wasm_string(&mut caller, &native_res.name)
                        )
                            as i64
                    };
                };
                // Write field Winner.size
                {
                    let size_out = winner_out as usize + std::mem::size_of::<i64>();
                    unsafe { *(mem_base.add(size_out as usize) as *mut f64) = native_res.size };
                };
                // Write field Winner.r#type
                {
                    let r#type_out = winner_out as usize
                        + std::mem::size_of::<i64>()
                        + std::mem::size_of::<f64>();
                    //Write enum Type
                    unsafe {
                        *(mem_base.add(r#type_out as usize) as *mut u8) = native_res.r#type as u8
                    };
                };
                // Write field Winner.data
                {
                    let data_out = winner_out as usize
                        + std::mem::size_of::<i64>()
                        + std::mem::size_of::<f64>()
                        + 1;
                    let data_out_len = winner_out as usize
                        + std::mem::size_of::<i64>()
                        + std::mem::size_of::<f64>()
                        + 1
                        + std::mem::size_of::<i64>();
                    let offset = try_or_errmsg!(
                        caller,
                        allocate(
                            &mut caller,
                            native_res.data.len() * std::mem::size_of::<u8>()
                        )
                    );

                    for (i, item) in native_res.data.iter().enumerate() {
                        let data_out = (i * std::mem::size_of::<u8>()) + offset;
                        unsafe { *(mem_base.add(data_out as usize) as *mut u8) = *item };
                    }
                    unsafe {
                        *(mem_base.add(data_out as usize) as *mut i64) = offset as i64;
                        *(mem_base.add(data_out_len as usize) as *mut i64) =
                            native_res.data.len() as i64;
                    };
                };
            }

            0
        }

        pub fn get_fish<T: super::OceanApi>(
            mut caller: wasmtime::Caller<'_, T>,
            name: i64,
            fish_out: i64,
        ) -> i64 {
            let mem = *try_or_errmsg!(caller, &get_memory(&mut caller, "memory"));
            let mem_base = mem.data_ptr(&caller);
            let data = caller.data_mut();

            // Convert input "name"
            let name = try_or_errmsg!(caller, get_string(mem_base, name as usize));

            let native_res = try_or_errmsg!(caller, data.get_fish(name));
            // TODO GENErATE THIS
            // Destruct native res
            let name = (native_res.name.as_ptr(), native_res.name.len());
            let name_len = (
                unsafe { mem_base.add(fish_out as usize + std::mem::size_of::<i64>()) as *mut i64 },
                native_res.name.len() as i64,
            );
            let size = (
                unsafe {
                    mem_base.add(
                        fish_out as usize + std::mem::size_of::<i64>() + std::mem::size_of::<i64>(),
                    ) as *mut f64
                },
                native_res.size,
            );
            let r#type = (
                unsafe {
                    mem_base.add(
                        fish_out as usize + std::mem::size_of::<i64>() + std::mem::size_of::<i64>() + std::mem::size_of::<f64>(),
                    ) as *mut u8
                },
                native_res.r#type as u8,
            );
            // Allocate
            let name = (
                unsafe { mem_base.add(fish_out as usize /*+ 0*/) as *mut i64 },
                try_or_errmsg!(caller, create_wasm_string2(&mut caller, name.0, name.1)) as i64,
            );

            // Write
            unsafe {
                *name.0 = name.1;
                *name_len.0 = name_len.1;
                *size.0 = size.1;
                *r#type.0 = r#type.1;
            }

            // Write output "fish"
            // Write record Fish
            /*{
                let size = native_res.size;
                let record_base = unsafe { mem_base.add(fish_out as usize) as *mut u8 };
                // Write field Fish.name
                {

                    let name_out = fish_out as usize;
                    unsafe {
                        *(mem_base.add(name_out as usize) as *mut i64) = try_or_errmsg!(
                            caller,
                            create_wasm_string2(&mut caller, name.0, name.1)
                        )
                            as i64
                    };
                };
                // Write field Fish.size
                {
                    let size_out = fish_out as usize  + std::mem::size_of::<i64>();
                    unsafe { *(mem_base.add( size_out as usize ) as *mut f64) = size };
                };
                // Write field Fish.r#type
                {
                    let r#type_out = fish_out as usize  + std::mem::size_of::<i64>() + std::mem::size_of::<f64>();
                    //Write enum Type
                    unsafe { *(mem_base.add( r#type_out as usize ) as *mut u8) = native_res.r#type as u8};
                };
                // Write field Fish.data
                {
                    let data_out = fish_out as usize  + std::mem::size_of::<i64>() + std::mem::size_of::<f64>() + 1;
                    let data_out_len = fish_out as usize  + std::mem::size_of::<i64>() + std::mem::size_of::<f64>() + 1 + std::mem::size_of::<i64>();
                    let offset = try_or_errmsg!(caller, allocate(&mut caller, native_res.data.len() * std::mem::size_of::<u8>()));

                    for (i, item) in native_res.data.iter().enumerate() {
                        let data_out = (i * std::mem::size_of::<u8>()) + offset;
                        unsafe { *(mem_base.add( data_out as usize ) as *mut u8) = *item };
                    }
                    unsafe {
                        *(mem_base.add( data_out as usize ) as *mut i64) = offset as i64;
                        *(mem_base.add( data_out_len as usize ) as *mut i64) = native_res.data.len() as i64;
                    };
                };
            }*/

            0
        }

        pub fn get_fishes_of_type<T: super::OceanApi>(
            mut caller: wasmtime::Caller<'_, T>,
            r#type: i32,
            fishes_out: i64,
            fishes_out_len: i64,
        ) -> i64 {
            let mem = *try_or_errmsg!(caller, &get_memory(&mut caller, "memory"));
            let mem_base = mem.data_ptr(&caller);
            let data = caller.data_mut();

            // Convert input "type"
            let r#type = try_or_errmsg!(caller, super::FishType::try_from(r#type as u8));

            let native_res = try_or_errmsg!(caller, data.get_fishes_of_type(r#type));

            // Write output "fishes"
            let offset =
                try_or_errmsg!(caller, allocate(&mut caller, native_res.len() * FISH_SIZE));

            for (i, item) in native_res.iter().enumerate() {
                let fishes_out = (i * FISH_SIZE) + offset;
                // Write record Fishes
                {
                    let record_base = unsafe { mem_base.add(fishes_out as usize) as *mut u8 };
                    // Write field Fishes.name
                    {
                        let name_out = fishes_out as usize;
                        unsafe {
                            *(mem_base.add(name_out as usize) as *mut i64) =
                                try_or_errmsg!(caller, create_wasm_string(&mut caller, &item.name))
                                    as i64
                        };
                    };
                    // Write field Fishes.size
                    {
                        let size_out = fishes_out as usize + std::mem::size_of::<i64>();
                        unsafe { *(mem_base.add(size_out as usize) as *mut f64) = item.size };
                    };
                    // Write field Fishes.r#type
                    {
                        let r#type_out = fishes_out as usize
                            + std::mem::size_of::<i64>()
                            + std::mem::size_of::<f64>();
                        //Write enum Type
                        unsafe {
                            *(mem_base.add(r#type_out as usize) as *mut u8) = item.r#type as u8
                        };
                    };
                    // Write field Fishes.data
                    {
                        let data_out = fishes_out as usize
                            + std::mem::size_of::<i64>()
                            + std::mem::size_of::<f64>()
                            + 1;
                        let data_out_len = fishes_out as usize
                            + std::mem::size_of::<i64>()
                            + std::mem::size_of::<f64>()
                            + 1
                            + std::mem::size_of::<i64>();
                        let offset = try_or_errmsg!(
                            caller,
                            allocate(&mut caller, item.data.len() * std::mem::size_of::<u8>())
                        );

                        for (i, item) in item.data.iter().enumerate() {
                            let data_out = (i * std::mem::size_of::<u8>()) + offset;
                            unsafe { *(mem_base.add(data_out as usize) as *mut u8) = *item };
                        }
                        unsafe {
                            *(mem_base.add(data_out as usize) as *mut i64) = offset as i64;
                            *(mem_base.add(data_out_len as usize) as *mut i64) =
                                item.data.len() as i64;
                        };
                    };
                }
            }
            unsafe {
                *(mem_base.add(fishes_out as usize) as *mut i64) = offset as i64;
                *(mem_base.add(fishes_out_len as usize) as *mut i64) = native_res.len() as i64;
            };
            0
        }
    }
}
