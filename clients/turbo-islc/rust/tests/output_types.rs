use turbo_isl::turbo;

turbo!("./tests/output-types.tisl" -> wasmtime);

struct HatTypes {}

impl hats::HatsApi for HatTypes {
    fn hat_is_funny(&mut self, name: &str) -> Result<bool, String> {
        Ok(["Propeller Cap", "Dunce Cap", "Party Hat", "Wizard Hat"].contains(&name))
    }

    /// Get the name of a hat, at random!
    fn random(&mut self, seed: i64) -> Result<String, String> {
        Ok(String::from(match seed {
            -400 => "Tricone",
            13 => "Šajkača",
            1007 => "Sombrero",
            1008 => "Bycocket",
            _ => "Top Hat",
        }))
    }

    fn size_to_diameter(&mut self, size: i64) -> Result<f64, String> {
        (size > 0)
            .then(|| (size as f64) * 13.5)
            .ok_or_else(|| String::from("Size has to be greater than zero!"))
    }

    fn diameter_to_size(&mut self, diameter: f64) -> Result<i64, String> {
        Ok((diameter.abs() / 13.5) as i64)
    }

    fn get_hat(&mut self, name: &'_ str) -> Result<hats::GetHatResult, String> {
        Ok(hats::GetHatResult {
            name: name.to_owned(),
            color: match name {
                "fez" => "red",
                _ => "grey",
            }.to_owned(),
        })
    }

    fn get_random_hat_names(
        &mut self,
        amount: i64,
    ) -> Result<Vec<String>, String> {
        let mut hats = vec![];
        for i in 0..amount {
            hats.push(self.random(i)?);
        }
        Ok(hats)
    }

    fn get_random_hats(
        &mut self,
        amount: i64,
    ) -> Result<Vec<hats::Hat>, String> {
        let mut hats: Vec<hats::Hat> = vec![];
        for i in 0..amount {
            let shelf = self.random(i)?;
            hats.push({
                let hat = self.get_hat(&shelf)?;
                hats::Hat {
                    name: hat.name,
                    color: hat.color,
                    water_proof: false,
                }
            })
        }
        Ok(hats)
    }
}

#[test]
fn test_output_types() {
    
}
