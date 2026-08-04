#![allow(unused)]

use std::borrow::Cow;

use crate::{hir::{body_map::Location, exprs::Name, types::{Fn, Primitive, Type}}, ir::def_map::DefId};
use Primitive::*;


pub type Symbol = &'static str;

// GLOBAL and BUILTIN OBJECTS/SYMBOLS
pub const MSG: Symbol = "msg";
pub const BLOCK: Symbol = "block";
pub const ABI: Symbol = "abi";
pub const TX: Symbol = "tx";


pub const THIS: Symbol = "this";
pub const SUPER: Symbol = "super";
pub const ADDRESS: Symbol = "address";
pub const UINT: Symbol = "uint";
pub const INT: Symbol = "int";
pub const STRING: Symbol = "string";
pub const BYTES: Symbol = "bytes";
pub const TYPE: Symbol = "type";




#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct BuiltinId(u8);

impl BuiltinId {
    pub fn name(&self) -> Symbol {
        match self {
            BuiltinId(0) => MSG,
            BuiltinId(1) => BLOCK,
            BuiltinId(2) => ABI,
            BuiltinId(3) => TX,
            _ => "unknown",
        }
    }

    pub fn doc(&self) -> &'static str {
        match self {
            BuiltinId(0) => "Message context of the current call",
            BuiltinId(1) => "Context of the current block",
            BuiltinId(2) => "ABI encoding and decoding functions",
            BuiltinId(3) => "Transaction context",
            _ => "",
        }
    }
}

pub struct BuiltinMemberId(u8);

#[derive(Clone, PartialEq, Eq)]
pub struct BuiltinField {
    pub name: Symbol,
    pub ty: Primitive,
    pub loc: Location,
    pub doc: Cow<'static, str>,
}

type ParamType = Cow<'static, [Type]>;

#[derive(Clone, PartialEq, Eq)]
pub struct BuiltinFn {
    pub name: Symbol,
    pub params: ParamType,
    pub variadic: bool,
    pub return_type: Option<Type>,
    pub doc: Cow<'static, str>,
}

#[derive(Clone, PartialEq, Eq)]
pub enum BuiltinMember {
    Field(BuiltinField),
    Fn(BuiltinFn),
}
use BuiltinMember::*;

static MSG_MEMBERS: [BuiltinMember; 4] = [
    Field(BuiltinField { name: "sender", ty: Address, loc: Location::Stack, doc: Cow::Borrowed("The sender of the message (current call)") }),
    Field(BuiltinField { name: "value", ty: Uint(256), loc: Location::Stack, doc: Cow::Borrowed("Number of wei sent with the message") }),
    Field(BuiltinField { name: "sig", ty: FixedBytes(4), loc: Location::Stack, doc: Cow::Borrowed("First four bytes of the calldata (function selector)") }),
    Field(BuiltinField { name: "data", ty: Bytes, loc: Location::Memory, doc: Cow::Borrowed("Complete calldata") }),
];

static BLOCK_MEMBERS: [BuiltinMember; 8] = [
    Field(BuiltinField { name: "number", ty: Uint(256), loc: Location::Stack, doc: Cow::Borrowed("Current block number") }),
    Field(BuiltinField { name: "timestamp", ty: Uint(256), loc: Location::Stack, doc: Cow::Borrowed("Current block timestamp (seconds since unix epoch)") }),
    Field(BuiltinField { name: "coinbase", ty: Address, loc: Location::Stack, doc: Cow::Borrowed("Current block miner's address") }),
    Field(BuiltinField { name: "gaslimit", ty: Uint(256), loc: Location::Stack, doc: Cow::Borrowed("Current block gas limit") }),
    Field(BuiltinField { name: "difficulty", ty: Uint(256), loc: Location::Stack, doc: Cow::Borrowed("Current block difficulty (superseded by prevrandao after the Merge)") }),
    Field(BuiltinField { name: "prevrandao", ty: Uint(256), loc: Location::Stack, doc: Cow::Borrowed("Randomness beacon output of the previous block") }),
    Field(BuiltinField { name: "chainid", ty: Uint(256), loc: Location::Stack, doc: Cow::Borrowed("ID of the current chain") }),
    Field(BuiltinField { name: "basefee", ty: Uint(256), loc: Location::Stack, doc: Cow::Borrowed("Current block's base fee") }),
];

static TX_MEMBERS: [BuiltinMember; 2] = [
    BuiltinMember::Field(BuiltinField { name: "origin", ty: Address, loc: Location::Stack, doc: Cow::Borrowed("Sender of the transaction (full call chain)") }),
    BuiltinMember::Field(BuiltinField { name: "gasprice", ty: Uint(256), loc: Location::Stack, doc: Cow::Borrowed("Gas price of the transaction") }),
];

static ABI_MEMBERS: [BuiltinMember; 6] = [
    Fn(BuiltinFn { name: "encode", params: Cow::Borrowed(&[]), variadic: true, return_type: Some(Type::Primitive(Bytes)), doc: Cow::Borrowed("ABI-encodes the given arguments") }),
    Fn(BuiltinFn { name: "encodePacked", params: Cow::Borrowed(&[]), variadic: true, return_type: Some(Type::Primitive(Bytes)), doc: Cow::Borrowed("ABI-encodes the given arguments with non-standard packed mode") }),
    Fn(BuiltinFn { name: "encodeWithSelector", params: Cow::Borrowed(&[Type::Primitive(FixedBytes(4))]), variadic: true, return_type: Some(Type::Primitive(Bytes)), doc: Cow::Borrowed("ABI-encodes with a 4-byte selector") }),
    Fn(BuiltinFn { name: "encodeWithSignature", params: Cow::Borrowed(&[Type::Primitive(String)]), variadic: true, return_type: Some(Type::Primitive(Bytes)), doc: Cow::Borrowed("ABI-encodes with a function signature") }),
    Fn(BuiltinFn { name: "encodeCall", params: Cow::Borrowed(&[]), variadic: true, return_type: Some(Type::Primitive(Bytes)), doc: Cow::Borrowed("ABI-encodes a function call") }),
    Fn(BuiltinFn { name: "decode", params: Cow::Borrowed(&[Type::Primitive(Bytes)]), variadic: true, return_type: None, doc: Cow::Borrowed("ABI-decodes the given data") }),
];

static ADDRESS_MEMBERS: [BuiltinMember; 5] = [
    Field(BuiltinField { name: "balance", ty: Uint(256), loc: Location::Stack, doc: Cow::Borrowed("Balance of the address in wei") }),
    Field(BuiltinField { name: "code", ty: Bytes, loc: Location::Memory, doc: Cow::Borrowed("Code at the address (can be empty)") }),
    Field(BuiltinField { name: "codehash", ty: FixedBytes(32), loc: Location::Stack, doc: Cow::Borrowed("The codehash of the address") }),
    Fn(BuiltinFn { name: "transfer", params: Cow::Borrowed(&[Type::Primitive(Uint(256))]), variadic: false, return_type: None, doc: Cow::Borrowed("Sends Ether to the address, reverts on failure") }),
    Fn(BuiltinFn { name: "send", params: Cow::Borrowed(&[Type::Primitive(Uint(256))]), variadic: false, return_type: Some(Type::Primitive(Boolean)), doc: Cow::Borrowed("Sends Ether to the address, returns false on failure") }),
];

static BYTES_MEMBERS: [BuiltinMember; 1] = [
    Fn(BuiltinFn { name: "concat", params: Cow::Borrowed(&[Type::Primitive(Bytes)]), variadic: true, return_type: Some(Type::Primitive(Bytes)), doc: Cow::Borrowed("Concatenates multiple bytes arrays") }),
];

static STRING_MEMBERS: [BuiltinMember; 1] = [
    Fn(BuiltinFn { name: "concat", params: Cow::Borrowed(&[Type::Primitive(String)]), variadic: true, return_type: Some(Type::Primitive(String)), doc: Cow::Borrowed("Concatenates multiple strings") }),
];

static FN_MEMBERS: [BuiltinMember; 2] = [
    Field(BuiltinField { name: "selector", ty: FixedBytes(4), loc: Location::Stack, doc: Cow::Borrowed("ABI selector of the function") }),
    Field(BuiltinField { name: "address", ty: Address, loc: Location::Stack, doc: Cow::Borrowed("The contract this function belongs to") }),
];

pub struct BuiltinDB;

impl BuiltinDB {
    pub fn resolve_name(name: &str) -> Option<BuiltinId> {
    
        match name {
            MSG => Some(BuiltinId(0)),
            BLOCK => Some(BuiltinId(1)),
            ABI => Some(BuiltinId(2)),
            TX => Some(BuiltinId(3)),
            _ => None,
        }
    }

    pub fn lookup_in_global(id: BuiltinId, name: &str) -> Option<BuiltinMember> {
        match id {
            BuiltinId(0) => find_member(&msg_members(), name),
            BuiltinId(1) => find_member(&block_members(), name),
            BuiltinId(2) => find_member(&abi_members(), name),
            BuiltinId(3) => find_member(&tx_members(), name),
            _ => None,
        }
    }

    pub fn lookup_in_type(ty: &Type, name: &str) -> Option<BuiltinMember> {
        match ty {
            Type::Primitive(Primitive::Address | Primitive::AddressPayable) => find_member(&address_members(), name),
            Type::Primitive(Primitive::Bytes) => find_member(&bytes_members(), name),
            Type::Primitive(Primitive::String) => find_member(&string_members(), name),
            Type::Array{ty,..} => find_member(&array_members(ty), name),
            Type::Fn(_) => find_member(&fn_members(), name),
            _ => None,
        }
    }

    pub fn lookup_in_udvt(def_id: DefId, ty: Name,  underlying: Primitive, name: &str) -> Option<BuiltinMember> {
        find_member(&udvt_members(def_id, ty, underlying), name)
    }

    pub fn lookup_in_meta(ty: &Type, name: &str) -> Option<BuiltinMember> {
        find_member(&type_members(ty), name)
    }
}

fn udvt_members(def_id: DefId, ty: Name, underlying: Primitive) -> Cow<'static, [BuiltinMember]> {
    let udvt_ty = Type::Def(def_id);
    let underlying_ty = Type::Primitive(underlying);
    Cow::Owned(vec![
        Fn(BuiltinFn { name: "wrap", params: Cow::Owned(vec![underlying_ty.clone()]), variadic: false, return_type: Some(udvt_ty.clone()), doc: format!("Wraps a {} value into {}", underlying, ty ).into()}),
        Fn(BuiltinFn { name: "unwrap", params: Cow::Owned(vec![udvt_ty]), variadic: false, return_type: Some(underlying_ty), doc: format!("Unwraps the {} into a {}", ty, underlying ).into() }),
    ])
}

fn msg_members() -> Cow<'static, [BuiltinMember]> {
    Cow::Borrowed(&MSG_MEMBERS)
}

fn block_members() -> Cow<'static, [BuiltinMember]> {
    Cow::Borrowed(&BLOCK_MEMBERS)
}

fn abi_members() -> Cow<'static, [BuiltinMember]> {
    Cow::Borrowed(&ABI_MEMBERS)
}

fn tx_members() -> Cow<'static, [BuiltinMember]> {
    Cow::Borrowed(&TX_MEMBERS)
}

fn fn_members() -> Cow<'static, [BuiltinMember]> {
    Cow::Borrowed(&FN_MEMBERS)
}

fn array_members(ty: &Type) -> Cow<'static, [BuiltinMember]> {
    Cow::Owned(vec![//FIXME:  Name search always returns the first push
        Field(BuiltinField { name: "length", ty: Uint(256), loc: Location::Stack, doc: Cow::Borrowed("Number of elements in the array") }),
        Fn(BuiltinFn { name: "push", params: Cow::Owned(vec![ty.clone()]), variadic: false, return_type: None, doc: Cow::Borrowed("Appends an element to the end of the array") }),
        Fn(BuiltinFn { name: "push", params: Cow::Borrowed(&[]), variadic: false, return_type: Some(ty.clone()), doc: Cow::Borrowed("Appends a new element and returns a reference to it") }),
        Fn(BuiltinFn { name: "pop", params: Cow::Borrowed(&[]), variadic: false, return_type: None, doc: Cow::Borrowed("Removes the last element from the array") }),
    ])
}

fn address_members() -> Cow<'static, [BuiltinMember]> {
    let mut members: Vec<BuiltinMember> = ADDRESS_MEMBERS.to_vec();
    let ret = Type::Tuple(Box::new([Type::Primitive(Boolean), Type::Primitive(Bytes)]));
    members.extend([
        Fn(BuiltinFn { name: "call", params: Cow::Borrowed(&[Type::Primitive(Bytes)]), variadic: false, return_type: Some(ret.clone()), doc: Cow::Borrowed("Low-level CALL with given calldata") }),
        Fn(BuiltinFn { name: "delegatecall", params: Cow::Borrowed(&[Type::Primitive(Bytes)]), variadic: false, return_type: Some(ret.clone()), doc: Cow::Borrowed("Low-level DELEGATECALL with given calldata") }),
        Fn(BuiltinFn { name: "staticcall", params: Cow::Borrowed(&[Type::Primitive(Bytes)]), variadic: false, return_type: Some(ret), doc: Cow::Borrowed("Low-level STATICCALL with given calldata") }),
    ]);
    Cow::Owned(members)
}

fn bytes_members() -> Cow<'static, [BuiltinMember]> {
    Cow::Borrowed(&BYTES_MEMBERS)
}

fn string_members() -> Cow<'static, [BuiltinMember]> {
    Cow::Borrowed(&STRING_MEMBERS)
}

/// type(T) members
fn type_members(ty: &Type) -> Cow<'static, [BuiltinMember]> {
    let mut members = vec![];

    match ty {
        Type::Primitive(Uint(n)) => {
            members.push(Field(BuiltinField { name: "min", ty: Uint(*n), loc: Location::Stack, doc: Cow::Borrowed("The minimum value representable by the type") }));
            members.push(Field(BuiltinField { name: "max", ty: Uint(*n), loc: Location::Stack, doc: Cow::Borrowed("The maximum value representable by the type") }));
        }
        Type::Primitive(Int(n)) => {
            members.push(Field(BuiltinField { name: "min", ty: Int(*n), loc: Location::Stack, doc: Cow::Borrowed("The minimum value representable by the type") }));
            members.push(Field(BuiltinField { name: "max", ty: Int(*n), loc: Location::Stack, doc: Cow::Borrowed("The maximum value representable by the type") }));
        }
        Type::Def(DefId::Enum(_)) => {
            members.push(Field(BuiltinField { name: "min", ty: Uint(256), loc: Location::Stack, doc: Cow::Borrowed("The smallest value of the enum") }));
            members.push(Field(BuiltinField { name: "max", ty: Uint(256), loc: Location::Stack, doc: Cow::Borrowed("The largest value of the enum") }));
        }
        Type::Def(DefId::Contract(_)) => {
            members.push(Field(BuiltinField { name: "name", ty: String, loc: Location::Memory, doc: Cow::Borrowed("Name of the contract") }));
            members.push(Field(BuiltinField { name: "creationCode", ty: Bytes, loc: Location::Memory, doc: Cow::Borrowed("Creation bytecode of the contract") }));
            members.push(Field(BuiltinField { name: "runtimeCode", ty: Bytes, loc: Location::Memory, doc: Cow::Borrowed("Runtime bytecode of the contract") }));
        }
        Type::Def(DefId::Interface(_)) => {
            members.push(Field(BuiltinField { name: "name", ty: String, loc: Location::Memory, doc: Cow::Borrowed("Name of the interface") }));
            members.push(Field(BuiltinField { name: "interfaceId", ty: Bytes, loc: Location::Memory, doc: Cow::Borrowed("EIP-165 interface identifier of the interface") }));
            members.push(Field(BuiltinField { name: "creationCode", ty: Bytes, loc: Location::Memory, doc: Cow::Borrowed("Creation bytecode of the interface") }));
            members.push(Field(BuiltinField { name: "runtimeCode", ty: Bytes, loc: Location::Memory, doc: Cow::Borrowed("Runtime bytecode of the interface") }));
        }
        Type::Def(DefId::Library(_)) => {
            members.push(Field(BuiltinField { name: "name", ty: String, loc: Location::Memory, doc: Cow::Borrowed("Name of the library") }));
            members.push(Field(BuiltinField { name: "creationCode", ty: Bytes, loc: Location::Memory, doc: Cow::Borrowed("Creation bytecode of the library") }));
            members.push(Field(BuiltinField { name: "runtimeCode", ty: Bytes, loc: Location::Memory, doc: Cow::Borrowed("Runtime bytecode of the library") }));
        }
        _ => {}
    }

    Cow::Owned(members)
}

fn find_member(members: &[BuiltinMember], name: &str) -> Option<BuiltinMember> {
    members.iter().find(|(BuiltinMember::Field(BuiltinField { name: n, .. }) |BuiltinMember::Fn(BuiltinFn { name: n,.. }) )| *n == name ).cloned()
}