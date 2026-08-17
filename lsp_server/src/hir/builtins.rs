use std::borrow::Cow;

use crate::{hir::{body_map::Location, exprs::Name, types::{Primitive, Type}}, ir::def_map::DefId};
use Primitive::*;


pub type Symbol = &'static str;
pub type ParamType = Cow<'static, [Type]>;


// GLOBAL and BUILTIN OBJECTS/SYMBOLS
pub const MSG: Symbol = "msg";
pub const BLOCK: Symbol = "block";
pub const ABI: Symbol = "abi";
pub const TX: Symbol = "tx";
pub const ASSERT: Symbol = "assert";
pub const REQUIRE: Symbol = "require";
pub const GASLEFT: Symbol = "gasleft";
pub const BLOCKHASH: Symbol = "blockhash";
pub const SELFDESTRUCT: Symbol = "selfdestruct";
pub const NOW: Symbol = "now";
pub const KECCAK256: Symbol = "keccak256";
pub const SHA256: Symbol = "sha256";
pub const RIPEMD160: Symbol = "ripemd160";
pub const ECRECOVER: Symbol = "ecrecover";
pub const ADDMOD: Symbol = "addmod";
pub const MULMOD: Symbol = "mulmod";
pub const THIS: Symbol = "this";
pub const SUPER: Symbol = "super";


#[derive(Clone, PartialEq, Eq)]
pub struct BuiltinObj {
    pub name: Symbol,
    pub doc: Cow<'static, str>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct BuiltinField {
    pub name: Symbol,
    pub ty: Primitive,
    pub loc: Location,
    pub doc: Cow<'static, str>,
}


#[derive(Clone, PartialEq, Eq)]
pub struct BuiltinFn {
    pub name: Symbol,
    pub params: ParamType,
    pub variadic: bool,
    pub return_type: Option<Type>,
    pub doc: Cow<'static, str>,
}


#[derive(Clone, PartialEq, Eq)]
pub enum Builtin {
    Obj(BuiltinObj),
    Fn(BuiltinFn),
    Field(BuiltinField),
}
use Builtin::*;

impl Builtin {
    pub fn name(&self) -> Symbol {
        match self {
            Obj(object) => object.name,
            Fn(function) => function.name,
            Field(field) => field.name,
        }
    }

    pub fn doc(&self) -> &str {
        match self {
            Obj(object) => &object.doc,
            Fn(function) => &function.doc,
            Field(field) => &field.doc,
        }
    }
}



static GLOBALS: [Builtin; 19] = [
    Obj(BuiltinObj { name: MSG, doc: Cow::Borrowed("Message context of the current call") }),
    Obj(BuiltinObj { name: BLOCK, doc: Cow::Borrowed("Context of the current block") }),
    Obj(BuiltinObj { name: ABI, doc: Cow::Borrowed("ABI encoding and decoding functions") }),
    Obj(BuiltinObj { name: TX, doc: Cow::Borrowed("Transaction context") }),
    Fn(BuiltinFn { name: ASSERT, params: Cow::Borrowed(&[Type::Primitive(Boolean)]), variadic: false, return_type: None, doc: Cow::Borrowed("Throws if the condition is false") }),
    Fn(BuiltinFn { name: REQUIRE, params: Cow::Borrowed(&[Type::Primitive(Boolean)]), variadic: false, return_type: None, doc: Cow::Borrowed("Reverts if the condition is false") }),
    Fn(BuiltinFn { name: REQUIRE, params: Cow::Borrowed(&[Type::Primitive(Boolean), Type::Primitive(String)]), variadic: false, return_type: None, doc: Cow::Borrowed("Reverts with a message if the condition is false") }),
    Fn(BuiltinFn { name: REQUIRE, params: Cow::Borrowed(&[Type::Primitive(Boolean), Type::Primitive(Bytes)]), variadic: false, return_type: None, doc: Cow::Borrowed("Reverts with encoded data if the condition is false") }),
    Fn(BuiltinFn { name: REQUIRE, params: Cow::Borrowed(&[Type::Primitive(Boolean), Type::Error]), variadic: false, return_type: None, doc: Cow::Borrowed("Reverts with a custom error if the condition is false") }),
    Fn(BuiltinFn { name: KECCAK256, params: Cow::Borrowed(&[Type::Primitive(Bytes)]), variadic: false, return_type: Some(Type::Primitive(FixedBytes(32))), doc: Cow::Borrowed("Computes the Keccak-256 hash") }),
    Fn(BuiltinFn { name: SHA256, params: Cow::Borrowed(&[Type::Primitive(Bytes)]), variadic: false, return_type: Some(Type::Primitive(FixedBytes(32))), doc: Cow::Borrowed("Computes the SHA-256 hash") }),
    Fn(BuiltinFn { name: RIPEMD160, params: Cow::Borrowed(&[Type::Primitive(Bytes)]), variadic: false, return_type: Some(Type::Primitive(FixedBytes(20))), doc: Cow::Borrowed("Computes the RIPEMD-160 hash") }),
    Fn(BuiltinFn { name: ECRECOVER, params: Cow::Borrowed(&[Type::Primitive(FixedBytes(32)), Type::Primitive(Uint(8)), Type::Primitive(FixedBytes(32)), Type::Primitive(FixedBytes(32))]), variadic: false, return_type: Some(Type::Primitive(Address)), doc: Cow::Borrowed("Recovers the address that signed a message") }),
    Fn(BuiltinFn { name: ADDMOD, params: Cow::Borrowed(&[Type::Primitive(Uint(256)), Type::Primitive(Uint(256)), Type::Primitive(Uint(256))]), variadic: false, return_type: Some(Type::Primitive(Uint(256))), doc: Cow::Borrowed("Computes addition modulo a modulus") }),
    Fn(BuiltinFn { name: MULMOD, params: Cow::Borrowed(&[Type::Primitive(Uint(256)), Type::Primitive(Uint(256)), Type::Primitive(Uint(256))]), variadic: false, return_type: Some(Type::Primitive(Uint(256))), doc: Cow::Borrowed("Computes multiplication modulo a modulus") }),
    Fn(BuiltinFn { name: GASLEFT, params: Cow::Borrowed(&[]), variadic: false, return_type: Some(Type::Primitive(Uint(256))), doc: Cow::Borrowed("Returns the remaining gas") }),
    Fn(BuiltinFn { name: BLOCKHASH, params: Cow::Borrowed(&[Type::Primitive(Uint(256))]), variadic: false, return_type: Some(Type::Primitive(FixedBytes(32))), doc: Cow::Borrowed("Returns the hash of a recent block") }),
    Fn(BuiltinFn { name: SELFDESTRUCT, params: Cow::Borrowed(&[Type::Primitive(AddressPayable)]), variadic: false, return_type: None, doc: Cow::Borrowed("Destroys the current contract") }),
    Fn(BuiltinFn { name: NOW, params: Cow::Borrowed(&[]), variadic: false, return_type: Some(Type::Primitive(Uint(256))), doc: Cow::Borrowed("Deprecated alias for block.timestamp") }),
];

static MSG_MEMBERS: [Builtin; 4] = [
    Field(BuiltinField { name: "sender", ty: Address, loc: Location::Stack, doc: Cow::Borrowed("The sender of the message (current call)") }),
    Field(BuiltinField { name: "value", ty: Uint(256), loc: Location::Stack, doc: Cow::Borrowed("Number of wei sent with the message") }),
    Field(BuiltinField { name: "sig", ty: FixedBytes(4), loc: Location::Stack, doc: Cow::Borrowed("First four bytes of the calldata (function selector)") }),
    Field(BuiltinField { name: "data", ty: Bytes, loc: Location::Memory, doc: Cow::Borrowed("Complete calldata") }),
];

static BLOCK_MEMBERS: [Builtin; 8] = [
    Field(BuiltinField { name: "number", ty: Uint(256), loc: Location::Stack, doc: Cow::Borrowed("Current block number") }),
    Field(BuiltinField { name: "timestamp", ty: Uint(256), loc: Location::Stack, doc: Cow::Borrowed("Current block timestamp (seconds since unix epoch)") }),
    Field(BuiltinField { name: "coinbase", ty: Address, loc: Location::Stack, doc: Cow::Borrowed("Current block miner's address") }),
    Field(BuiltinField { name: "gaslimit", ty: Uint(256), loc: Location::Stack, doc: Cow::Borrowed("Current block gas limit") }),
    Field(BuiltinField { name: "difficulty", ty: Uint(256), loc: Location::Stack, doc: Cow::Borrowed("Current block difficulty (superseded by prevrandao after the Merge)") }),
    Field(BuiltinField { name: "prevrandao", ty: Uint(256), loc: Location::Stack, doc: Cow::Borrowed("Randomness beacon output of the previous block") }),
    Field(BuiltinField { name: "chainid", ty: Uint(256), loc: Location::Stack, doc: Cow::Borrowed("ID of the current chain") }),
    Field(BuiltinField { name: "basefee", ty: Uint(256), loc: Location::Stack, doc: Cow::Borrowed("Current block's base fee") }),
];

static TX_MEMBERS: [Builtin; 2] = [
    Builtin::Field(BuiltinField { name: "origin", ty: Address, loc: Location::Stack, doc: Cow::Borrowed("Sender of the transaction (full call chain)") }),
    Builtin::Field(BuiltinField { name: "gasprice", ty: Uint(256), loc: Location::Stack, doc: Cow::Borrowed("Gas price of the transaction") }),
];


static ABI_MEMBERS: [Builtin; 6] = [
    Fn(BuiltinFn { name: "encode", params: Cow::Borrowed(&[]), variadic: true, return_type: Some(Type::Primitive(Bytes)), doc: Cow::Borrowed("ABI-encodes the given arguments") }),
    Fn(BuiltinFn { name: "encodePacked", params: Cow::Borrowed(&[]), variadic: true, return_type: Some(Type::Primitive(Bytes)), doc: Cow::Borrowed("ABI-encodes the given arguments with non-standard packed mode") }),
    Fn(BuiltinFn { name: "encodeWithSelector", params: Cow::Borrowed(&[Type::Primitive(FixedBytes(4))]), variadic: true, return_type: Some(Type::Primitive(Bytes)), doc: Cow::Borrowed("ABI-encodes with a 4-byte selector") }),
    Fn(BuiltinFn { name: "encodeWithSignature", params: Cow::Borrowed(&[Type::Primitive(String)]), variadic: true, return_type: Some(Type::Primitive(Bytes)), doc: Cow::Borrowed("ABI-encodes with a function signature") }),
    Fn(BuiltinFn { name: "encodeCall", params: Cow::Borrowed(&[]), variadic: true, return_type: Some(Type::Primitive(Bytes)), doc: Cow::Borrowed("ABI-encodes a function call") }),
    Fn(BuiltinFn { name: "decode", params: Cow::Borrowed(&[Type::Primitive(Bytes)]), variadic: true, return_type: None, doc: Cow::Borrowed("ABI-decodes the given data") }),
];

static ADDRESS_MEMBERS: [Builtin; 5] = [
    Field(BuiltinField { name: "balance", ty: Uint(256), loc: Location::Stack, doc: Cow::Borrowed("Balance of the address in wei") }),
    Field(BuiltinField { name: "code", ty: Bytes, loc: Location::Memory, doc: Cow::Borrowed("Code at the address (can be empty)") }),
    Field(BuiltinField { name: "codehash", ty: FixedBytes(32), loc: Location::Stack, doc: Cow::Borrowed("The codehash of the address") }),
    Fn(BuiltinFn { name: "transfer", params: Cow::Borrowed(&[Type::Primitive(Uint(256))]), variadic: false, return_type: None, doc: Cow::Borrowed("Sends Ether to the address, reverts on failure") }),
    Fn(BuiltinFn { name: "send", params: Cow::Borrowed(&[Type::Primitive(Uint(256))]), variadic: false, return_type: Some(Type::Primitive(Boolean)), doc: Cow::Borrowed("Sends Ether to the address, returns false on failure") }),
];

static BYTES_MEMBERS: [Builtin; 1] = [
    Fn(BuiltinFn { name: "concat", params: Cow::Borrowed(&[Type::Primitive(Bytes)]), variadic: true, return_type: Some(Type::Primitive(Bytes)), doc: Cow::Borrowed("Concatenates multiple bytes arrays") }),
];

static STRING_MEMBERS: [Builtin; 1] = [
    Fn(BuiltinFn { name: "concat", params: Cow::Borrowed(&[Type::Primitive(String)]), variadic: true, return_type: Some(Type::Primitive(String)), doc: Cow::Borrowed("Concatenates multiple strings") }),
];

static FN_MEMBERS: [Builtin; 2] = [
    Field(BuiltinField { name: "selector", ty: FixedBytes(4), loc: Location::Stack, doc: Cow::Borrowed("ABI selector of the function") }),
    Field(BuiltinField { name: "address", ty: Address, loc: Location::Stack, doc: Cow::Borrowed("The contract this function belongs to") }),
];

static ERROR_MEMBERS: [Builtin; 1] = [
    Field(BuiltinField { name: "selector", ty: FixedBytes(4), loc: Location::Stack, doc: Cow::Borrowed("ABI selector of the custom error") }),
];



// MARK: BuiltinDB
pub struct BuiltinDB;

impl BuiltinDB {
    pub fn resolve_name(name: &str) -> Vec<Builtin> {
        GLOBALS.iter()
            .filter_map(|global| match global {
                Obj(object) if object.name == name => Some(Obj(object.clone())),
                Fn(function) if function.name == name => Some(Fn(function.clone())),
                _ => None,
            })
            .collect()
    }

    pub fn globals() -> &'static [Builtin] {
        &GLOBALS
    }

    pub fn members_in_global(global: &Builtin) -> Cow<'static, [Builtin]> {
        match global.name() {
            MSG => Cow::Borrowed(&MSG_MEMBERS),
            BLOCK => Cow::Borrowed(&BLOCK_MEMBERS),
            ABI => Cow::Borrowed(&ABI_MEMBERS),
            TX => Cow::Borrowed(&TX_MEMBERS),
            _ => Cow::Borrowed(&[]),
        }
    }

    fn find_member(members: &[Builtin], name: &str) -> Option<Builtin> {
        members.iter().find(|member| member.name() == name).cloned()
    }

    pub fn lookup_in_global(global: &Builtin, name: &str) -> Option<Builtin> {
        Self::find_member(&Self::members_in_global(global), name)
    }
    
    pub fn members_in_type(ty: &Type) -> Cow<'static, [Builtin]> {
        match ty {
            Type::Primitive(Primitive::Address | Primitive::AddressPayable) => Self::address_members(),
            Type::Primitive(Primitive::Bytes) => Cow::Borrowed(&BYTES_MEMBERS),
            Type::Primitive(Primitive::String) => Cow::Borrowed(&STRING_MEMBERS),
            Type::Array { ty, .. } => Self::array_members(ty),
            Type::Fn(_) => Cow::Borrowed(&FN_MEMBERS),
            Type::Error => Cow::Borrowed(&ERROR_MEMBERS),
            _ => Cow::Borrowed(&[]),
        }
    }

    pub fn lookup_in_type(ty: &Type, name: &str) -> Option<Builtin> {
        Self::find_member(&Self::members_in_type(ty), name)
    }

    pub fn lookup_in_udvt(def_id: DefId, ty: Name, underlying: Primitive, name: &str) -> Option<Builtin> {
        Self::find_member(&Self::udvt_members(def_id, ty, underlying), name)
    }

    pub fn lookup_in_meta(ty: &Type, name: &str) -> Option<Builtin> {
        Self::find_member(&Self::meta_type_members(ty), name)
    }


    pub fn udvt_members(def_id: DefId, ty: Name, underlying: Primitive) -> Cow<'static, [Builtin]> {
        let udvt_ty = Type::Def(def_id);
        let underlying_ty = Type::Primitive(underlying);
        Cow::Owned(vec![
            Fn(BuiltinFn { name: "wrap", params: Cow::Owned(vec![underlying_ty.clone()]), variadic: false, return_type: Some(udvt_ty.clone()), doc: format!("Wraps a {} value into {}", underlying, ty ).into()}),
            Fn(BuiltinFn { name: "unwrap", params: Cow::Owned(vec![udvt_ty]), variadic: false, return_type: Some(underlying_ty), doc: format!("Unwraps the {} into a {}", ty, underlying ).into() }),
        ])
    }


    fn array_members(ty: &Type) -> Cow<'static, [Builtin]> {
        Cow::Owned(vec![//FIXME:  Name search always returns the first push
            Field(BuiltinField { name: "length", ty: Uint(256), loc: Location::Stack, doc: Cow::Borrowed("Number of elements in the array") }),
            Fn(BuiltinFn { name: "push", params: Cow::Owned(vec![ty.clone()]), variadic: false, return_type: None, doc: Cow::Borrowed("Appends an element to the end of the array") }),
            Fn(BuiltinFn { name: "push", params: Cow::Borrowed(&[]), variadic: false, return_type: Some(ty.clone()), doc: Cow::Borrowed("Appends a new element and returns a reference to it") }),
            Fn(BuiltinFn { name: "pop", params: Cow::Borrowed(&[]), variadic: false, return_type: None, doc: Cow::Borrowed("Removes the last element from the array") }),
        ])
    }

    fn address_members() -> Cow<'static, [Builtin]> {
        let mut members: Vec<Builtin> = ADDRESS_MEMBERS.to_vec();
        let ret = Type::Tuple(Box::new([Type::Primitive(Boolean), Type::Primitive(Bytes)]));
        members.extend([
            Fn(BuiltinFn { name: "call", params: Cow::Borrowed(&[Type::Primitive(Bytes)]), variadic: false, return_type: Some(ret.clone()), doc: Cow::Borrowed("Low-level CALL with given calldata") }),
            Fn(BuiltinFn { name: "delegatecall", params: Cow::Borrowed(&[Type::Primitive(Bytes)]), variadic: false, return_type: Some(ret.clone()), doc: Cow::Borrowed("Low-level DELEGATECALL with given calldata") }),
            Fn(BuiltinFn { name: "staticcall", params: Cow::Borrowed(&[Type::Primitive(Bytes)]), variadic: false, return_type: Some(ret), doc: Cow::Borrowed("Low-level STATICCALL with given calldata") }),
        ]);
        Cow::Owned(members)
    }


    /// type(T) members
    pub fn meta_type_members(ty: &Type) -> Cow<'static, [Builtin]> {
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
            Type::Def(DefId::Enum(_)) => {//@TODO: enum min/max are variants not uint
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
}



