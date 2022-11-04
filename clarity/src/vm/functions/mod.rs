// Copyright (C) 2013-2020 Blockstack PBC, a public benefit corporation
// Copyright (C) 2020 Stacks Open Internet Foundation
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <http://www.gnu.org/licenses/>.

use crate::vm::callables::{CallableType, NativeHandle};
use crate::vm::costs::cost_functions::ClarityCostFunction;
use crate::vm::costs::{
    constants as cost_constants, cost_functions, runtime_cost, CostTracker, MemoryConsumer,
};
use crate::vm::errors::{
    check_argument_count, check_arguments_at_least, CheckErrors, Error,
    InterpreterResult as Result, RuntimeErrorType, ShortReturnType,
};
pub use crate::vm::functions::assets::stx_transfer_consolidated;
use crate::vm::is_reserved;
use crate::vm::representations::SymbolicExpressionType::{Atom, List};
use crate::vm::representations::{ClarityName, SymbolicExpression, SymbolicExpressionType};
use crate::vm::types::{
    BuffData, CharType, PrincipalData, ResponseData, SequenceData, TypeSignature, Value, BUFF_32,
    BUFF_33, BUFF_65,
};
use crate::vm::{eval, Environment, LocalContext};
use stacks_common::address::AddressHashMode;
use stacks_common::util::hash;

use crate::types::chainstate::StacksAddress;
use crate::vm::callables::cost_input_sized_vararg;

macro_rules! switch_on_global_epoch {
    ($Name:ident ($Epoch2Version:ident, $Epoch205Version:ident)) => {
        pub fn $Name(
            args: &[SymbolicExpression],
            env: &mut Environment,
            context: &LocalContext,
        ) -> Result<Value> {
            match env.epoch() {
                StacksEpochId::Epoch10 => {
                    panic!("Executing Clarity method during Epoch 1.0, before Clarity")
                }
                StacksEpochId::Epoch20 => $Epoch2Version(args, env, context),
                StacksEpochId::Epoch2_05 => $Epoch205Version(args, env, context),
                // Note: We reuse 2.05 for 2.1.
                StacksEpochId::Epoch21 => $Epoch205Version(args, env, context),
            }
        }
    };
}

use crate::vm::ClarityVersion;

mod arithmetic;
mod assets;
mod boolean;
mod conversions;
mod crypto;
mod database;
pub mod define;
mod options;
pub mod principals;
mod sequences;
pub mod tuples;

define_versioned_named_enum_for_clarity_fns!(NativeFunctions(ClarityVersion, ClarityCostFunction) {
    Add("+", ClarityVersion::Clarity1, ClarityCostFunction::Add),
    Subtract("-", ClarityVersion::Clarity1, ClarityCostFunction::Sub),
    Multiply("*", ClarityVersion::Clarity1, ClarityCostFunction::Mul),
    Divide("/", ClarityVersion::Clarity1, ClarityCostFunction::Geq),
    CmpGeq(">=", ClarityVersion::Clarity1, ClarityCostFunction::Leq),
    CmpLeq("<=", ClarityVersion::Clarity1, ClarityCostFunction::Add),
    CmpLess("<", ClarityVersion::Clarity1, ClarityCostFunction::Le),
    CmpGreater(">", ClarityVersion::Clarity1, ClarityCostFunction::Ge),
    ToInt("to-int", ClarityVersion::Clarity1, ClarityCostFunction::IntCast),
    ToUInt("to-uint", ClarityVersion::Clarity1, ClarityCostFunction::IntCast),
    Modulo("mod", ClarityVersion::Clarity1, ClarityCostFunction::Mod),
    Power("pow", ClarityVersion::Clarity1, ClarityCostFunction::Pow),
    Sqrti("sqrti", ClarityVersion::Clarity1, ClarityCostFunction::Sqrti),
    Log2("log2", ClarityVersion::Clarity1, ClarityCostFunction::Log2),
    BitwiseXOR("xor", ClarityVersion::Clarity1, ClarityCostFunction::Xor),
    And("and", ClarityVersion::Clarity1, ClarityCostFunction::And),
    Or("or", ClarityVersion::Clarity1, ClarityCostFunction::Or),
    Not("not", ClarityVersion::Clarity1, ClarityCostFunction::Not),
    Equals("is-eq", ClarityVersion::Clarity1, ClarityCostFunction::Eq),
    If("if", ClarityVersion::Clarity1, ClarityCostFunction::If),
    Let("let", ClarityVersion::Clarity1, ClarityCostFunction::Let),
    Map("map", ClarityVersion::Clarity1, ClarityCostFunction::Map),
    Fold("fold", ClarityVersion::Clarity1, ClarityCostFunction::Fold),
    Append("append", ClarityVersion::Clarity1, ClarityCostFunction::Append),
    Concat("concat", ClarityVersion::Clarity1, ClarityCostFunction::Concat),
    AsMaxLen("as-max-len?", ClarityVersion::Clarity1, ClarityCostFunction::AsMaxLen),
    Len("len", ClarityVersion::Clarity1, ClarityCostFunction::Len),
    ElementAt("element-at", ClarityVersion::Clarity1, ClarityCostFunction::ElementAt),
    IndexOf("index-of", ClarityVersion::Clarity1, ClarityCostFunction::IndexOf),
    BuffToIntLe("buff-to-int-le", ClarityVersion::Clarity2, ClarityCostFunction::BuffToIntLe),
    BuffToUIntLe("buff-to-uint-le", ClarityVersion::Clarity2, ClarityCostFunction::BuffToUIntLe),
    BuffToIntBe("buff-to-int-be", ClarityVersion::Clarity2, ClarityCostFunction::BuffToIntBe),
    BuffToUIntBe("buff-to-uint-be", ClarityVersion::Clarity2, ClarityCostFunction::BuffToUIntBe),
    IsStandard("is-standard", ClarityVersion::Clarity2, ClarityCostFunction::IsStandard),
    PrincipalDestruct("principal-destruct", ClarityVersion::Clarity2, ClarityCostFunction::PrincipalDestruct),
    PrincipalConstruct("principal-construct", ClarityVersion::Clarity2, ClarityCostFunction::PrincipalConstruct),
    StringToInt("string-to-int", ClarityVersion::Clarity2, ClarityCostFunction::StringToInt),
    StringToUInt("string-to-uint", ClarityVersion::Clarity2, ClarityCostFunction::StringToUInt),
    IntToAscii("int-to-ascii", ClarityVersion::Clarity2, ClarityCostFunction::IntToAscii),
    IntToUtf8("int-to-utf8", ClarityVersion::Clarity2, ClarityCostFunction::IntToUtf8),
    ListCons("list", ClarityVersion::Clarity1, ClarityCostFunction::ListCons),
    FetchVar("var-get", ClarityVersion::Clarity1, ClarityCostFunction::FetchVar),
    SetVar("var-set", ClarityVersion::Clarity1, ClarityCostFunction::SetVar),
    FetchEntry("map-get?", ClarityVersion::Clarity1, ClarityCostFunction::FetchEntry),
    SetEntry("map-set", ClarityVersion::Clarity1, ClarityCostFunction::SetEntry),
    // InsertEntry re-uses cost function for SetEntry
    InsertEntry("map-insert", ClarityVersion::Clarity1, ClarityCostFunction::SetEntry),
    // DeleteEntry re-uses cost function for SetEntry
    DeleteEntry("map-delete", ClarityVersion::Clarity1, ClarityCostFunction::SetEntry),
    TupleCons("tuple", ClarityVersion::Clarity1, ClarityCostFunction::TupleCons),
    TupleGet("get", ClarityVersion::Clarity1, ClarityCostFunction::TupleGet),
    TupleMerge("merge", ClarityVersion::Clarity1, ClarityCostFunction::TupleMerge),
    Begin("begin", ClarityVersion::Clarity1, ClarityCostFunction::Begin),
    Hash160("hash160", ClarityVersion::Clarity1, ClarityCostFunction::Hash160),
    Sha256("sha256", ClarityVersion::Clarity1, ClarityCostFunction::Sha256),
    Sha512("sha512", ClarityVersion::Clarity1, ClarityCostFunction::Sha512),
    Sha512Trunc256("sha512/256", ClarityVersion::Clarity1, ClarityCostFunction::Sha512t256),
    Keccak256("keccak256", ClarityVersion::Clarity1, ClarityCostFunction::Keccak256),
    Secp256k1Recover("secp256k1-recover?", ClarityVersion::Clarity1, ClarityCostFunction::Secp256k1recover),
    Secp256k1Verify("secp256k1-verify", ClarityVersion::Clarity1, ClarityCostFunction::Secp256k1verify),
    Print("print", ClarityVersion::Clarity1, ClarityCostFunction::Print),
    ContractCall("contract-call?", ClarityVersion::Clarity1, ClarityCostFunction::ContractCall),
    // AsContract does not have an associated ClarityCostFunction
    AsContract("as-contract", ClarityVersion::Clarity1, ClarityCostFunction::Unimplemented),
    ContractOf("contract-of", ClarityVersion::Clarity1, ClarityCostFunction::ContractOf),
    PrincipalOf("principal-of?", ClarityVersion::Clarity1, ClarityCostFunction::PrincipalOf),
    AtBlock("at-block", ClarityVersion::Clarity1, ClarityCostFunction::AtBlock),
    GetBlockInfo("get-block-info?", ClarityVersion::Clarity1, ClarityCostFunction::BlockInfo),
    GetBurnBlockInfo("get-burn-block-info?", ClarityVersion::Clarity2, ClarityCostFunction::GetBurnBlockInfo),
    ConsError("err", ClarityVersion::Clarity1, ClarityCostFunction::ErrCons),
    ConsOkay("ok", ClarityVersion::Clarity1, ClarityCostFunction::OkCons),
    ConsSome("some", ClarityVersion::Clarity1, ClarityCostFunction::SomeCons),
    DefaultTo("default-to", ClarityVersion::Clarity1, ClarityCostFunction::DefaultTo),
    Asserts("asserts!", ClarityVersion::Clarity1, ClarityCostFunction::Asserts),
    UnwrapRet("unwrap!", ClarityVersion::Clarity1, ClarityCostFunction::UnwrapRet),
    UnwrapErrRet("unwrap-err!", ClarityVersion::Clarity1, ClarityCostFunction::UnwrapErrOrRet),
    Unwrap("unwrap-panic", ClarityVersion::Clarity1, ClarityCostFunction::Unwrap),
    UnwrapErr("unwrap-err-panic", ClarityVersion::Clarity1, ClarityCostFunction::UnwrapErr),
    Match("match", ClarityVersion::Clarity1, ClarityCostFunction::Match),
    TryRet("try!", ClarityVersion::Clarity1, ClarityCostFunction::TryRet),
    IsOkay("is-ok", ClarityVersion::Clarity1, ClarityCostFunction::IsOkay),
    IsNone("is-none", ClarityVersion::Clarity1, ClarityCostFunction::IsNone),
    IsErr("is-err", ClarityVersion::Clarity1, ClarityCostFunction::IsErr),
    IsSome("is-some", ClarityVersion::Clarity1, ClarityCostFunction::IsSome),
    Filter("filter", ClarityVersion::Clarity1, ClarityCostFunction::Filter),
    GetTokenBalance("ft-get-balance", ClarityVersion::Clarity1, ClarityCostFunction::FtBalance),
    GetAssetOwner("nft-get-owner?", ClarityVersion::Clarity1, ClarityCostFunction::NftOwner),
    TransferToken("ft-transfer?", ClarityVersion::Clarity1, ClarityCostFunction::FtTransfer),
    TransferAsset("nft-transfer?", ClarityVersion::Clarity1, ClarityCostFunction::NftTransfer),
    MintAsset("nft-mint?", ClarityVersion::Clarity1, ClarityCostFunction::NftMint),
    MintToken("ft-mint?", ClarityVersion::Clarity1, ClarityCostFunction::FtMint),
    GetTokenSupply("ft-get-supply", ClarityVersion::Clarity1, ClarityCostFunction::FtSupply),
    BurnToken("ft-burn?", ClarityVersion::Clarity1, ClarityCostFunction::FtBurn),
    BurnAsset("nft-burn?", ClarityVersion::Clarity1, ClarityCostFunction::NftBurn),
    GetStxBalance("stx-get-balance", ClarityVersion::Clarity1, ClarityCostFunction::StxBalance),
    StxTransfer("stx-transfer?", ClarityVersion::Clarity1, ClarityCostFunction::StxTransfer),
    StxTransferMemo("stx-transfer-memo?", ClarityVersion::Clarity2, ClarityCostFunction::StxTransferMemo),
    // StxBurn re-uses cost function for StxTransfer
    StxBurn("stx-burn?", ClarityVersion::Clarity1, ClarityCostFunction::StxTransfer),
    StxGetAccount("stx-account", ClarityVersion::Clarity2, ClarityCostFunction::StxGetAccount),
    Slice("slice", ClarityVersion::Clarity2, ClarityCostFunction::Slice),
    ToConsensusBuff("to-consensus-buff", ClarityVersion::Clarity2, ClarityCostFunction::ToConsensusBuff),
    FromConsensusBuff("from-consensus-buff", ClarityVersion::Clarity2, ClarityCostFunction::FromConsensusBuff),
    ReplaceAt("replace-at", ClarityVersion::Clarity2, ClarityCostFunction::ReplaceAt),
});

impl NativeFunctions {
    pub fn lookup_by_name_at_version(
        name: &str,
        version: &ClarityVersion,
    ) -> Option<NativeFunctions> {
        NativeFunctions::lookup_by_name(name).and_then(|native_function| {
            if &native_function.get_version() <= version {
                Some(native_function)
            } else {
                None
            }
        })
    }
}

///
/// Returns a callable for the given native function if it exists in the provided
///   ClarityVersion
///
pub fn lookup_reserved_functions(name: &str, version: &ClarityVersion) -> Option<CallableType> {
    use crate::vm::callables::CallableType::{NativeFunction, NativeFunction205, SpecialFunction};
    use crate::vm::functions::NativeFunctions::*;
    if let Some(native_function) = NativeFunctions::lookup_by_name_at_version(name, version) {
        let callable = match native_function {
            Add => NativeFunction(
                "native_add",
                NativeHandle::MoreArg(&arithmetic::native_add),
                ClarityCostFunction::Add,
            ),
            Subtract => NativeFunction(
                "native_sub",
                NativeHandle::MoreArg(&arithmetic::native_sub),
                ClarityCostFunction::Sub,
            ),
            Multiply => NativeFunction(
                "native_mul",
                NativeHandle::MoreArg(&arithmetic::native_mul),
                ClarityCostFunction::Mul,
            ),
            Divide => NativeFunction(
                "native_div",
                NativeHandle::MoreArg(&arithmetic::native_div),
                ClarityCostFunction::Div,
            ),
            CmpGeq => SpecialFunction("special_geq", &arithmetic::special_geq),
            CmpLeq => SpecialFunction("special_leq", &arithmetic::special_leq),
            CmpLess => SpecialFunction("special_le", &arithmetic::special_less),
            CmpGreater => SpecialFunction("special_ge", &arithmetic::special_greater),
            ToUInt => NativeFunction(
                "native_to_uint",
                NativeHandle::SingleArg(&arithmetic::native_to_uint),
                ClarityCostFunction::IntCast,
            ),
            ToInt => NativeFunction(
                "native_to_int",
                NativeHandle::SingleArg(&arithmetic::native_to_int),
                ClarityCostFunction::IntCast,
            ),
            Modulo => NativeFunction(
                "native_mod",
                NativeHandle::DoubleArg(&arithmetic::native_mod),
                ClarityCostFunction::Mod,
            ),
            Power => NativeFunction(
                "native_pow",
                NativeHandle::DoubleArg(&arithmetic::native_pow),
                ClarityCostFunction::Pow,
            ),
            Sqrti => NativeFunction(
                "native_sqrti",
                NativeHandle::SingleArg(&arithmetic::native_sqrti),
                ClarityCostFunction::Sqrti,
            ),
            Log2 => NativeFunction(
                "native_log2",
                NativeHandle::SingleArg(&arithmetic::native_log2),
                ClarityCostFunction::Log2,
            ),
            BitwiseXOR => NativeFunction(
                "native_xor",
                NativeHandle::DoubleArg(&arithmetic::native_xor),
                ClarityCostFunction::Xor,
            ),
            And => SpecialFunction("special_and", &boolean::special_and),
            Or => SpecialFunction("special_or", &boolean::special_or),
            Not => NativeFunction(
                "native_not",
                NativeHandle::SingleArg(&boolean::native_not),
                ClarityCostFunction::Not,
            ),
            Equals => NativeFunction205(
                "native_eq",
                NativeHandle::MoreArg(&native_eq),
                ClarityCostFunction::Eq,
                &cost_input_sized_vararg,
            ),
            If => SpecialFunction("special_if", &special_if),
            Let => SpecialFunction("special_let", &special_let),
            FetchVar => SpecialFunction("special_var-get", &database::special_fetch_variable),
            SetVar => SpecialFunction("special_set-var", &database::special_set_variable),
            Map => SpecialFunction("special_map", &sequences::special_map),
            Filter => SpecialFunction("special_filter", &sequences::special_filter),
            BuffToIntLe => NativeFunction(
                "native_buff_to_int_le",
                NativeHandle::SingleArg(&conversions::native_buff_to_int_le),
                ClarityCostFunction::Unimplemented,
            ),
            BuffToUIntLe => NativeFunction(
                "native_buff_to_uint_le",
                NativeHandle::SingleArg(&conversions::native_buff_to_uint_le),
                ClarityCostFunction::Unimplemented,
            ),
            BuffToIntBe => NativeFunction(
                "native_buff_to_int_be",
                NativeHandle::SingleArg(&conversions::native_buff_to_int_be),
                ClarityCostFunction::Unimplemented,
            ),
            BuffToUIntBe => NativeFunction(
                "native_buff_to_uint_be",
                NativeHandle::SingleArg(&conversions::native_buff_to_uint_be),
                ClarityCostFunction::Unimplemented,
            ),
            StringToInt => NativeFunction(
                "native_string_to_int",
                NativeHandle::SingleArg(&conversions::native_string_to_int),
                ClarityCostFunction::Unimplemented,
            ),
            StringToUInt => NativeFunction(
                "native_string_to_uint",
                NativeHandle::SingleArg(&conversions::native_string_to_uint),
                ClarityCostFunction::Unimplemented,
            ),
            IntToAscii => NativeFunction(
                "native_int_to_ascii",
                NativeHandle::SingleArg(&conversions::native_int_to_ascii),
                ClarityCostFunction::Unimplemented,
            ),
            IntToUtf8 => NativeFunction(
                "native_int_to_utf8",
                NativeHandle::SingleArg(&conversions::native_int_to_utf8),
                ClarityCostFunction::Unimplemented,
            ),
            IsStandard => SpecialFunction("special_is_standard", &principals::special_is_standard),
            PrincipalDestruct => SpecialFunction(
                "special_principal_destruct",
                &principals::special_principal_destruct,
            ),
            PrincipalConstruct => SpecialFunction(
                "special_principal_construct",
                &principals::special_principal_construct,
            ),
            Fold => SpecialFunction("special_fold", &sequences::special_fold),
            Concat => SpecialFunction("special_concat", &sequences::special_concat),
            AsMaxLen => SpecialFunction("special_as_max_len", &sequences::special_as_max_len),
            Append => SpecialFunction("special_append", &sequences::special_append),
            Len => NativeFunction(
                "native_len",
                NativeHandle::SingleArg(&sequences::native_len),
                ClarityCostFunction::Len,
            ),
            ElementAt => NativeFunction(
                "native_element_at",
                NativeHandle::DoubleArg(&sequences::native_element_at),
                ClarityCostFunction::ElementAt,
            ),
            IndexOf => NativeFunction205(
                "native_index_of",
                NativeHandle::DoubleArg(&sequences::native_index_of),
                ClarityCostFunction::IndexOf,
                &cost_input_sized_vararg,
            ),
            Slice => SpecialFunction("special_slice", &sequences::special_slice),
            ListCons => SpecialFunction("special_list_cons", &sequences::list_cons),
            FetchEntry => SpecialFunction("special_map-get?", &database::special_fetch_entry),
            SetEntry => SpecialFunction("special_set-entry", &database::special_set_entry),
            InsertEntry => SpecialFunction("special_insert-entry", &database::special_insert_entry),
            DeleteEntry => SpecialFunction("special_delete-entry", &database::special_delete_entry),
            TupleCons => SpecialFunction("special_tuple", &tuples::tuple_cons),
            TupleGet => SpecialFunction("special_get-tuple", &tuples::tuple_get),
            TupleMerge => NativeFunction205(
                "native_merge-tuple",
                NativeHandle::DoubleArg(&tuples::tuple_merge),
                ClarityCostFunction::TupleMerge,
                &cost_input_sized_vararg,
            ),
            Begin => NativeFunction(
                "native_begin",
                NativeHandle::MoreArg(&native_begin),
                ClarityCostFunction::Begin,
            ),
            Hash160 => NativeFunction205(
                "native_hash160",
                NativeHandle::SingleArg(&crypto::native_hash160),
                ClarityCostFunction::Hash160,
                &cost_input_sized_vararg,
            ),
            Sha256 => NativeFunction205(
                "native_sha256",
                NativeHandle::SingleArg(&crypto::native_sha256),
                ClarityCostFunction::Sha256,
                &cost_input_sized_vararg,
            ),
            Sha512 => NativeFunction205(
                "native_sha512",
                NativeHandle::SingleArg(&crypto::native_sha512),
                ClarityCostFunction::Sha512,
                &cost_input_sized_vararg,
            ),
            Sha512Trunc256 => NativeFunction205(
                "native_sha512trunc256",
                NativeHandle::SingleArg(&crypto::native_sha512trunc256),
                ClarityCostFunction::Sha512t256,
                &cost_input_sized_vararg,
            ),
            Keccak256 => NativeFunction205(
                "native_keccak256",
                NativeHandle::SingleArg(&crypto::native_keccak256),
                ClarityCostFunction::Keccak256,
                &cost_input_sized_vararg,
            ),
            Secp256k1Recover => SpecialFunction(
                "native_secp256k1-recover",
                &crypto::special_secp256k1_recover,
            ),
            Secp256k1Verify => {
                SpecialFunction("native_secp256k1-verify", &crypto::special_secp256k1_verify)
            }
            Print => SpecialFunction("special_print", &special_print),
            ContractCall => {
                SpecialFunction("special_contract-call", &database::special_contract_call)
            }
            AsContract => SpecialFunction("special_as-contract", &special_as_contract),
            ContractOf => SpecialFunction("special_contract-of", &special_contract_of),
            PrincipalOf => SpecialFunction("special_principal-of", &crypto::special_principal_of),
            GetBlockInfo => {
                SpecialFunction("special_get_block_info", &database::special_get_block_info)
            }
            GetBurnBlockInfo => SpecialFunction(
                "special_get_burn_block_info",
                &database::special_get_burn_block_info,
            ),
            ConsSome => NativeFunction(
                "native_some",
                NativeHandle::SingleArg(&options::native_some),
                ClarityCostFunction::SomeCons,
            ),
            ConsOkay => NativeFunction(
                "native_okay",
                NativeHandle::SingleArg(&options::native_okay),
                ClarityCostFunction::OkCons,
            ),
            ConsError => NativeFunction(
                "native_error",
                NativeHandle::SingleArg(&options::native_error),
                ClarityCostFunction::ErrCons,
            ),
            DefaultTo => NativeFunction(
                "native_default_to",
                NativeHandle::DoubleArg(&options::native_default_to),
                ClarityCostFunction::DefaultTo,
            ),
            Asserts => SpecialFunction("special_asserts", &special_asserts),
            UnwrapRet => NativeFunction(
                "native_unwrap_ret",
                NativeHandle::DoubleArg(&options::native_unwrap_or_ret),
                ClarityCostFunction::UnwrapRet,
            ),
            UnwrapErrRet => NativeFunction(
                "native_unwrap_err_ret",
                NativeHandle::DoubleArg(&options::native_unwrap_err_or_ret),
                ClarityCostFunction::UnwrapErrOrRet,
            ),
            IsOkay => NativeFunction(
                "native_is_okay",
                NativeHandle::SingleArg(&options::native_is_okay),
                ClarityCostFunction::IsOkay,
            ),
            IsNone => NativeFunction(
                "native_is_none",
                NativeHandle::SingleArg(&options::native_is_none),
                ClarityCostFunction::IsNone,
            ),
            IsErr => NativeFunction(
                "native_is_err",
                NativeHandle::SingleArg(&options::native_is_err),
                ClarityCostFunction::IsErr,
            ),
            IsSome => NativeFunction(
                "native_is_some",
                NativeHandle::SingleArg(&options::native_is_some),
                ClarityCostFunction::IsSome,
            ),
            Unwrap => NativeFunction(
                "native_unwrap",
                NativeHandle::SingleArg(&options::native_unwrap),
                ClarityCostFunction::Unwrap,
            ),
            UnwrapErr => NativeFunction(
                "native_unwrap_err",
                NativeHandle::SingleArg(&options::native_unwrap_err),
                ClarityCostFunction::UnwrapErr,
            ),
            Match => SpecialFunction("special_match", &options::special_match),
            TryRet => NativeFunction(
                "native_try_ret",
                NativeHandle::SingleArg(&options::native_try_ret),
                ClarityCostFunction::TryRet,
            ),
            MintAsset => SpecialFunction("special_mint_asset", &assets::special_mint_asset),
            MintToken => SpecialFunction("special_mint_token", &assets::special_mint_token),
            TransferAsset => {
                SpecialFunction("special_transfer_asset", &assets::special_transfer_asset)
            }
            TransferToken => {
                SpecialFunction("special_transfer_token", &assets::special_transfer_token)
            }
            GetTokenBalance => SpecialFunction("special_get_balance", &assets::special_get_balance),
            GetAssetOwner => SpecialFunction("special_get_owner", &assets::special_get_owner),
            BurnAsset => SpecialFunction("special_burn_asset", &assets::special_burn_asset),
            BurnToken => SpecialFunction("special_burn_token", &assets::special_burn_token),
            GetTokenSupply => SpecialFunction(
                "special_get_token_supply",
                &assets::special_get_token_supply,
            ),
            AtBlock => SpecialFunction("special_at_block", &database::special_at_block),
            GetStxBalance => SpecialFunction("special_stx_balance", &assets::special_stx_balance),
            StxTransfer => SpecialFunction("special_stx_transfer", &assets::special_stx_transfer),
            StxTransferMemo => SpecialFunction(
                "special_stx_transfer_memo",
                &assets::special_stx_transfer_memo,
            ),
            StxBurn => SpecialFunction("special_stx_burn", &assets::special_stx_burn),
            StxGetAccount => SpecialFunction("stx_get_account", &assets::special_stx_account),
            ToConsensusBuff => NativeFunction205(
                "to_consensus_buff",
                NativeHandle::SingleArg(&conversions::to_consensus_buff),
                ClarityCostFunction::Unimplemented,
                &cost_input_sized_vararg,
            ),
            FromConsensusBuff => {
                SpecialFunction("from_consensus_buff", &conversions::from_consensus_buff)
            }
            ReplaceAt => SpecialFunction("replace_at", &sequences::special_replace_at),
        };
        Some(callable)
    } else {
        None
    }
}

fn native_eq(args: Vec<Value>) -> Result<Value> {
    // TODO: this currently uses the derived equality checks of Value,
    //   however, that's probably not how we want to implement equality
    //   checks on the ::ListTypes

    if args.len() < 2 {
        Ok(Value::Bool(true))
    } else {
        let first = &args[0];
        // check types:
        let mut arg_type = TypeSignature::type_of(first);
        for x in args.iter() {
            arg_type = TypeSignature::least_supertype(&TypeSignature::type_of(x), &arg_type)?;
            if x != first {
                return Ok(Value::Bool(false));
            }
        }
        Ok(Value::Bool(true))
    }
}

fn native_begin(mut args: Vec<Value>) -> Result<Value> {
    match args.pop() {
        Some(v) => Ok(v),
        None => Err(CheckErrors::RequiresAtLeastArguments(1, 0).into()),
    }
}

fn special_print(
    args: &[SymbolicExpression],
    env: &mut Environment,
    context: &LocalContext,
) -> Result<Value> {
    let input = eval(&args[0], env, context)?;

    runtime_cost(ClarityCostFunction::Print, env, input.size())?;

    if cfg!(feature = "developer-mode") {
        info!("{}", &input);
    }

    env.register_print_event(input.clone())?;
    Ok(input)
}

fn special_if(
    args: &[SymbolicExpression],
    env: &mut Environment,
    context: &LocalContext,
) -> Result<Value> {
    check_argument_count(3, args)?;

    runtime_cost(ClarityCostFunction::If, env, 0)?;
    // handle the conditional clause.
    let conditional = eval(&args[0], env, context)?;
    match conditional {
        Value::Bool(result) => {
            if result {
                eval(&args[1], env, context)
            } else {
                eval(&args[2], env, context)
            }
        }
        _ => Err(CheckErrors::TypeValueError(TypeSignature::BoolType, conditional).into()),
    }
}

fn special_asserts(
    args: &[SymbolicExpression],
    env: &mut Environment,
    context: &LocalContext,
) -> Result<Value> {
    check_argument_count(2, args)?;

    runtime_cost(ClarityCostFunction::Asserts, env, 0)?;
    // handle the conditional clause.
    let conditional = eval(&args[0], env, context)?;

    match conditional {
        Value::Bool(result) => {
            if result {
                Ok(conditional)
            } else {
                let thrown = eval(&args[1], env, context)?;
                Err(ShortReturnType::AssertionFailed(thrown).into())
            }
        }
        _ => Err(CheckErrors::TypeValueError(TypeSignature::BoolType, conditional).into()),
    }
}

pub fn handle_binding_list<F, E>(
    bindings: &[SymbolicExpression],
    mut handler: F,
) -> std::result::Result<(), E>
where
    F: FnMut(&ClarityName, &SymbolicExpression) -> std::result::Result<(), E>,
    E: From<CheckErrors>,
{
    for binding in bindings.iter() {
        let binding_expression = binding.match_list().ok_or(CheckErrors::BadSyntaxBinding)?;
        if binding_expression.len() != 2 {
            return Err(CheckErrors::BadSyntaxBinding.into());
        }
        let var_name = binding_expression[0]
            .match_atom()
            .ok_or(CheckErrors::BadSyntaxBinding)?;
        let var_sexp = &binding_expression[1];

        handler(var_name, var_sexp)?;
    }
    Ok(())
}

pub fn parse_eval_bindings(
    bindings: &[SymbolicExpression],
    env: &mut Environment,
    context: &LocalContext,
) -> Result<Vec<(ClarityName, Value)>> {
    let mut result = Vec::new();
    handle_binding_list(bindings, |var_name, var_sexp| {
        eval(var_sexp, env, context).and_then(|value| {
            result.push((var_name.clone(), value));
            Ok(())
        })
    })?;

    Ok(result)
}

fn special_let(
    args: &[SymbolicExpression],
    env: &mut Environment,
    context: &LocalContext,
) -> Result<Value> {
    // (let ((x 1) (y 2)) (+ x y)) -> 3
    // arg0 => binding list
    // arg1..n => body
    check_arguments_at_least(2, args)?;

    // parse and eval the bindings.
    let bindings = args[0].match_list().ok_or(CheckErrors::BadLetSyntax)?;

    runtime_cost(ClarityCostFunction::Let, env, bindings.len())?;

    // create a new context.
    let mut inner_context = context.extend()?;

    let mut memory_use = 0;

    finally_drop_memory!( env, memory_use; {
        handle_binding_list::<_, Error>(bindings, |binding_name, var_sexp| {
            if is_reserved(binding_name, env.contract_context.get_clarity_version()) ||
                env.contract_context.lookup_function(binding_name).is_some() ||
                inner_context.lookup_variable(binding_name).is_some() {
                    return Err(CheckErrors::NameAlreadyUsed(binding_name.clone().into()).into())
                }

            let binding_value = eval(var_sexp, env, &inner_context)?;

            let bind_mem_use = binding_value.get_memory_use();
            env.add_memory(bind_mem_use)?;
            memory_use += bind_mem_use; // no check needed, b/c it's done in add_memory.
            inner_context.variables.insert(binding_name.clone(), binding_value);
            Ok(())
        })?;

        // evaluate the let-bodies
        let mut last_result = None;
        for body in args[1..].iter() {
            let body_result = eval(&body, env, &inner_context)?;
            last_result.replace(body_result);
        }
        // last_result should always be Some(...), because of the arg len check above.
        Ok(last_result.unwrap())
    })
}

fn special_as_contract(
    args: &[SymbolicExpression],
    env: &mut Environment,
    context: &LocalContext,
) -> Result<Value> {
    // (as-contract (..))
    // arg0 => body
    check_argument_count(1, args)?;

    // nest an environment.
    env.add_memory(cost_constants::AS_CONTRACT_MEMORY)?;

    let contract_principal = env.contract_context.contract_identifier.clone().into();
    let mut nested_env = env.nest_as_principal(contract_principal);

    let result = eval(&args[0], &mut nested_env, context);

    env.drop_memory(cost_constants::AS_CONTRACT_MEMORY);

    result
}

fn special_contract_of(
    args: &[SymbolicExpression],
    env: &mut Environment,
    context: &LocalContext,
) -> Result<Value> {
    // (contract-of (..))
    // arg0 => trait
    check_argument_count(1, args)?;

    runtime_cost(ClarityCostFunction::ContractOf, env, 0)?;

    let contract_ref = match &args[0].expr {
        SymbolicExpressionType::Atom(contract_ref) => contract_ref,
        _ => return Err(CheckErrors::ContractOfExpectsTrait.into()),
    };

    let contract_identifier = match context.lookup_callable_contract(contract_ref) {
        Some((ref contract_identifier, _trait_identifier)) => {
            env.global_context
                .database
                .get_contract(contract_identifier)
                .map_err(|_e| CheckErrors::NoSuchContract(contract_identifier.to_string()))?;

            contract_identifier
        }
        _ => return Err(CheckErrors::ContractOfExpectsTrait.into()),
    };

    let contract_principal = Value::Principal(PrincipalData::Contract(contract_identifier.clone()));
    Ok(contract_principal)
}
