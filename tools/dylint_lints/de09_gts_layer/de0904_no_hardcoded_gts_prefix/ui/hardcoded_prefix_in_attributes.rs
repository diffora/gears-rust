//! A hard-coded `gts.` prefix is rejected in every ordinary attribute.

#[doc = "gts.cf.de0904.tests.doc.v1~"]
struct Documented;

#[allow(dead_code, reason = "gts.cf.de0904.tests.reason.v1~")]
struct Allowed;

fn main() {
    let _ = (Documented, Allowed);
}
