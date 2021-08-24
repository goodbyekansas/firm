// TODO: Ugly hack to ensure we get windows messasges and resources into our exectuable
#![allow(dead_code)]
fn depend_on_winlog() {
    winlog::WinLogger::new("hack");
}
