#![feature(plugin)]
#![plugin(cpp)]
#![allow(unused)]

use std::ffi::CString;
use std::ptr;
use std::marker::PhantomData;

cpp_include!(<vector>);

fn add<T>(x:T, y:T) -> T { 
    unsafe {
        let z = cpp!((x,y) { return x+y; });
        z
    }
}

fn main() {
    let x = add(1,2);
    let y = add(1.0, 2.0);
    println!("{} {}", x, y);
}
