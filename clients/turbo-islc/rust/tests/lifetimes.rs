#[path = "./util.rs"]
mod util;

use std::collections::HashMap;

use turbo_isl::turbo;

use util::WasmString;

turbo!("./tests/lifetimes.tisl" -> wasmtime);

#[derive(Clone)]
pub struct Attachment {
    pub name: String,
    pub path: String,
    pub publisher: attackments::Human,
    pub authors: Vec<attackments::Human>,
}

impl<'a> From<&'a Attachment> for attackments::Attachment<'a> {
    fn from(att: &'a Attachment) -> Self {
        Self {
            name: att.name.clone(),
            path: att.path.clone(),
            publisher: &att.publisher,
            authors: att.authors.as_slice(),
        }
    }
}

impl attackments::Attachment<'_> {
    fn to_owned(&self) -> Attachment {
        Attachment {
            name: self.name.to_owned(),
            path: self.path.to_owned(),
            publisher: self.publisher.clone(),
            authors: self.authors.to_owned(),
        }
    }
}

#[derive(Clone)]
struct State {
    attachments: HashMap<String, Attachment>,
}

impl State {
    fn default() -> Self {
        Self {
            attachments: HashMap::<String, Attachment>::new(),
        }
    }
}

impl attackments::AttackmentsApi for State {
    fn get_attachment(&mut self, name: &str) -> Result<attackments::Attachment<'_>, String> {
        Ok(self.attachments.get(name).unwrap().into())
    }

    fn add_attachment(&mut self, attachment: &attackments::Attachment) -> Result<(), String> {
        self.attachments
            .insert(attachment.name.clone(), attachment.to_owned());
        Ok(())
    }
}

#[test]
fn test_lifetimes() {
    // Initialzing test environment
    let mut context = util::WasmTestContext::new(5, State::default());

    unsafe {
        context.setup_mock_functions(&mut attackments::GET_FUNCTION, &mut attackments::GET_MEMORY);
    }

    assert!(
        attackments::add_to_linker(&mut context.linker).is_ok(),
        "Expected to be able to add attackments module to linker."
    );

    // Grab the function we want to call
    let func = context.get_function("attackments", "add_attachment");

    let func =
        func.expect("Expected add_attachment to be a function inside the attackments module.");

    let result = context.call_function(
        func,
        // TODO make it into Val
        &[attackments::Attachment {
            name: String::from("fina-filen"),
            path: String::from("/p/a/t/h"),
            publisher: &attackments::Human {
                name: String::from("Arthur, King of the Britons"),
                shoe_size: 13,
                heart: attackments::Heart { bpm: 4 },
            },
            authors: &[
                attackments::Human {
                    name: String::from("Sir Lancelot the Brave"),
                    shoe_size: 7,
                    heart: attackments::Heart { bpm: 20000 },
                },
                attackments::Human {
                    name: String::from("Sir Galahad the Pure"),
                    shoe_size: 99,
                    heart: attackments::Heart { bpm: 404 },
                },
                attackments::Human {
                    name: String::from("Sir Robin the-not-quite-so-brave-as-Sir-Lancelot"),
                    shoe_size: 69,
                    heart: attackments::Heart { bpm: 420 },
                },
            ],
        }],
    );

    assert!(result.is_ok());
    // Maybe assert that it is in the state

    let func = context
        .get_function("attackments", "get_attachment")
        .expect("Tried to get get_attachment from attackments");

    let attachment_name = WasmString::from_str(context.mem_base, &context.allocator, "fina-filen");

    let attachment_result = context.allocator.allocate_ptr(); // TODO allocate correct thing?

    let result = context.call_function(func, &[attachment_name.into(), attachment_result.into()]);
}
