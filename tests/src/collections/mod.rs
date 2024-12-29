mod btreemap;
mod hashmap;
mod hashset;
mod linked_list;
mod vec;

macro_rules! redefine {
    (<$origin_ty:ty> :: $func_ident:ident => $($param_ident:ident : $param_ty:ty),* => $ret_ty:ty) => {
        fn $func_ident($($param_ident : $param_ty),*) -> $ret_ty {
            <$origin_ty>::$func_ident($($param_ident),*)
        }
    };
    (<$origin_ty:ty> :: $func_ident:ident<$($lt:lifetime),+> => $($param_ident:ident : $param_ty:ty),* => $ret_ty:ty) => {
        fn $func_ident<$($lt),*>($($param_ident : $param_ty),*) -> $ret_ty {
            <$origin_ty>::$func_ident($($param_ident),*)
        }
    };
    ($new_ident:ident, <$origin_ty:ty> :: $func_ident:ident => $($param_ident:ident : $param_ty:ty),* => $ret_ty:ty) => {
        fn $new_ident($($param_ident : $param_ty),*) -> $ret_ty {
            <$origin_ty>::$func_ident($($param_ident),*)
        }
    };
    ($new_ident:ident, <$origin_ty:ty> :: $func_ident:ident<$($lt:lifetime),+> => $($param_ident:ident : $param_ty:ty),* => $ret_ty:ty) => {
        fn $new_ident<$($lt),*>($($param_ident : $param_ty),*) -> $ret_ty {
            <$origin_ty>::$func_ident($($param_ident),*)
        }
    };
}

pub(crate) use redefine;
