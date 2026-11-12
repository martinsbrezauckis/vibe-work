//! Exact pane/document scroll requests and position reads; called by `browser_act` and the checked script assembler.
use serde_json::{Value, json};
/// Bounded scroll deltas, optionally bound to an observed scroll pane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    reference: Option<String>,
    x: i32,
    y: i32,
    position: bool,
}
impl Request {
    pub(crate) fn parse(args: &Value) -> Result<Self, String> {
        let reference = args
            .get("ref")
            .map(|v| {
                v.as_str()
                    .filter(|s| crate::model::is_ref(s))
                    .map(str::to_owned)
                    .ok_or(
                        "Scroll needs a current ref. Read the page and choose the intended pane.",
                    )
            })
            .transpose()?;
        let position = args["action"] == "scroll_position";
        let deltas = args.get("x").is_some() || args.get("y").is_some();
        if position
            && ["x", "y", "direction", "amount"]
                .iter()
                .any(|k| args.get(k).is_some())
        {
            return Err("A position read does not scroll. Omit deltas and direction.".into());
        }
        let delta = |key: &str| {
            args.get(key).map_or(Some(0),Value::as_i64).filter(|n|(-20000..=20000).contains(n)).map(|n|i32::try_from(n).unwrap_or_default()).ok_or_else(||"Scroll deltas must be integers from -20000 to 20000. Choose bounded x and y values.".to_owned())
        };
        let (x, y) = if deltas {
            if args.get("direction").is_some() || args.get("amount").is_some() {
                return Err(
                    "Scroll accepts deltas or direction and amount. Choose one form.".into(),
                );
            }
            (delta("x")?, delta("y")?)
        } else if position {
            (0, 0)
        } else {
            let amount = args
                .get("amount")
                .map_or(Some(600), Value::as_i64)
                .filter(|n| (1..=20000).contains(n))
                .and_then(|n| i32::try_from(n).ok())
                .ok_or("Scroll amount must be 1–20000 pixels. Choose a bounded amount.")?;
            match args["direction"].as_str().unwrap_or("down") {"up"=>(0,-amount),"down"=>(0,amount),"left"=>(-amount,0),"right"=>(amount,0),_=>return Err("Scroll direction must be up, down, left or right. Choose a supported direction.".into())}
        };
        Ok(Self {
            reference,
            x,
            y,
            position,
        })
    }
    pub(crate) fn reference(&self) -> Option<&str> {
        self.reference.as_deref()
    }
    pub(crate) fn name(&self) -> &'static str {
        if self.position {
            "scroll_position"
        } else {
            "scroll"
        }
    }
    pub(crate) fn args(&self) -> Value {
        json!({"ref":self.reference,"x":self.x,"y":self.y,"scrollMotion":true})
    }
}
