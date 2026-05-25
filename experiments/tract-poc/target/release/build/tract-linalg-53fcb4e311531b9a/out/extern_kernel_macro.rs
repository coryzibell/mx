
    macro_rules! extern_kernel {
        (fn $name: ident($($par_name:ident : $par_type: ty ),*) -> $rv: ty) => {
            paste! {
                unsafe extern "C" { pub fn [<$name _ 0_22_1>]($(par_name: $par_type),*) -> $rv; }
                pub use [<$name _ 0_22_1>] as $name;
            }
        }
    }