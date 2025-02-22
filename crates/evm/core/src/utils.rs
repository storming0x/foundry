use alloy_json_abi::{Function, JsonAbi};
use alloy_primitives::FixedBytes;
use alloy_rpc_types::{Block, Transaction};
use ethers::types::{ActionType, CallType, Chain, H256, U256};
use eyre::ContextCompat;
use foundry_common::types::ToAlloy;
use revm::{
    interpreter::{CallScheme, InstructionResult},
    primitives::{CreateScheme, Eval, Halt, SpecId, TransactTo},
};
use serde::{Deserialize, Serialize};

pub use foundry_compilers::utils::RuntimeOrHandle;
pub use revm::primitives::State as StateChangeset;

pub use crate::ic::*;

// TODO(onbjerg): Remove this and use `CallKind` from the tracer.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
#[derive(Default)]
pub enum CallKind {
    #[default]
    Call,
    StaticCall,
    CallCode,
    DelegateCall,
    Create,
    Create2,
}

impl From<CallScheme> for CallKind {
    fn from(scheme: CallScheme) -> Self {
        match scheme {
            CallScheme::Call => CallKind::Call,
            CallScheme::StaticCall => CallKind::StaticCall,
            CallScheme::CallCode => CallKind::CallCode,
            CallScheme::DelegateCall => CallKind::DelegateCall,
        }
    }
}

impl From<CreateScheme> for CallKind {
    fn from(create: CreateScheme) -> Self {
        match create {
            CreateScheme::Create => CallKind::Create,
            CreateScheme::Create2 { .. } => CallKind::Create2,
        }
    }
}

impl From<CallKind> for ActionType {
    fn from(kind: CallKind) -> Self {
        match kind {
            CallKind::Call | CallKind::StaticCall | CallKind::DelegateCall | CallKind::CallCode => {
                ActionType::Call
            }
            CallKind::Create => ActionType::Create,
            CallKind::Create2 => ActionType::Create,
        }
    }
}

impl From<CallKind> for CallType {
    fn from(ty: CallKind) -> Self {
        match ty {
            CallKind::Call => CallType::Call,
            CallKind::StaticCall => CallType::StaticCall,
            CallKind::CallCode => CallType::CallCode,
            CallKind::DelegateCall => CallType::DelegateCall,
            CallKind::Create => CallType::None,
            CallKind::Create2 => CallType::None,
        }
    }
}

/// Small helper function to convert [U256] into [H256].
#[inline]
pub fn u256_to_h256_le(u: U256) -> H256 {
    let mut h = H256::default();
    u.to_little_endian(h.as_mut());
    h
}

/// Small helper function to convert [U256] into [H256].
#[inline]
pub fn u256_to_h256_be(u: U256) -> H256 {
    let mut h = H256::default();
    u.to_big_endian(h.as_mut());
    h
}

/// Small helper function to convert [H256] into [U256].
#[inline]
pub fn h256_to_u256_be(storage: H256) -> U256 {
    U256::from_big_endian(storage.as_bytes())
}

/// Small helper function to convert [H256] into [U256].
#[inline]
pub fn h256_to_u256_le(storage: H256) -> U256 {
    U256::from_little_endian(storage.as_bytes())
}

/// Small helper function to convert an Eval into an InstructionResult
#[inline]
pub fn eval_to_instruction_result(eval: Eval) -> InstructionResult {
    match eval {
        Eval::Return => InstructionResult::Return,
        Eval::Stop => InstructionResult::Stop,
        Eval::SelfDestruct => InstructionResult::SelfDestruct,
    }
}

/// Small helper function to convert a Halt into an InstructionResult
#[inline]
pub fn halt_to_instruction_result(halt: Halt) -> InstructionResult {
    match halt {
        Halt::OutOfGas(_) => InstructionResult::OutOfGas,
        Halt::OpcodeNotFound => InstructionResult::OpcodeNotFound,
        Halt::InvalidFEOpcode => InstructionResult::InvalidFEOpcode,
        Halt::InvalidJump => InstructionResult::InvalidJump,
        Halt::NotActivated => InstructionResult::NotActivated,
        Halt::StackOverflow => InstructionResult::StackOverflow,
        Halt::StackUnderflow => InstructionResult::StackUnderflow,
        Halt::OutOfOffset => InstructionResult::OutOfOffset,
        Halt::CreateCollision => InstructionResult::CreateCollision,
        Halt::PrecompileError => InstructionResult::PrecompileError,
        Halt::NonceOverflow => InstructionResult::NonceOverflow,
        Halt::CreateContractSizeLimit => InstructionResult::CreateContractSizeLimit,
        Halt::CreateContractStartingWithEF => InstructionResult::CreateContractStartingWithEF,
        Halt::CreateInitcodeSizeLimit => InstructionResult::CreateInitcodeSizeLimit,
        Halt::OverflowPayment => InstructionResult::OverflowPayment,
        Halt::StateChangeDuringStaticCall => InstructionResult::StateChangeDuringStaticCall,
        Halt::CallNotAllowedInsideStatic => InstructionResult::CallNotAllowedInsideStatic,
        Halt::OutOfFund => InstructionResult::OutOfFund,
        Halt::CallTooDeep => InstructionResult::CallTooDeep,
        Halt::FailedDeposit => InstructionResult::Return,
    }
}

/// Depending on the configured chain id and block number this should apply any specific changes
///
/// This checks for:
///    - prevrandao mixhash after merge
pub fn apply_chain_and_block_specific_env_changes(env: &mut revm::primitives::Env, block: &Block) {
    if let Ok(chain) = Chain::try_from(env.cfg.chain_id) {
        let block_number = block.header.number.unwrap_or_default();

        match chain {
            Chain::Mainnet => {
                // after merge difficulty is supplanted with prevrandao EIP-4399
                if block_number.to::<u64>() >= 15_537_351u64 {
                    env.block.difficulty = env.block.prevrandao.unwrap_or_default().into();
                }

                return;
            }
            Chain::Arbitrum |
            Chain::ArbitrumGoerli |
            Chain::ArbitrumNova |
            Chain::ArbitrumTestnet => {
                // on arbitrum `block.number` is the L1 block which is included in the
                // `l1BlockNumber` field
                if let Some(l1_block_number) = block.other.get("l1BlockNumber").cloned() {
                    if let Ok(l1_block_number) = serde_json::from_value::<U256>(l1_block_number) {
                        env.block.number = l1_block_number.to_alloy();
                    }
                }
            }
            _ => {}
        }
    }

    // if difficulty is `0` we assume it's past merge
    if block.header.difficulty.is_zero() {
        env.block.difficulty = env.block.prevrandao.unwrap_or_default().into();
    }
}

/// Given an ABI and selector, it tries to find the respective function.
pub fn get_function(
    contract_name: &str,
    selector: &FixedBytes<4>,
    abi: &JsonAbi,
) -> eyre::Result<Function> {
    abi.functions()
        .find(|func| func.selector().as_slice() == selector.as_slice())
        .cloned()
        .wrap_err(format!("{contract_name} does not have the selector {selector:?}"))
}

/// Configures the env for the transaction
pub fn configure_tx_env(env: &mut revm::primitives::Env, tx: &Transaction) {
    env.tx.caller = tx.from;
    env.tx.gas_limit = tx.gas.to();
    env.tx.gas_price = tx.gas_price.unwrap_or_default().to();
    env.tx.gas_priority_fee = tx.max_priority_fee_per_gas.map(|g| g.to());
    env.tx.nonce = Some(tx.nonce.to());
    env.tx.access_list = tx
        .access_list
        .clone()
        .unwrap_or_default()
        .into_iter()
        .map(|item| {
            (
                item.address,
                item.storage_keys
                    .into_iter()
                    .map(|key| alloy_primitives::U256::from_be_bytes(key.0))
                    .collect(),
            )
        })
        .collect();
    env.tx.value = tx.value.to();
    env.tx.data = alloy_primitives::Bytes(tx.input.0.clone());
    env.tx.transact_to = tx.to.map(TransactTo::Call).unwrap_or_else(TransactTo::create)
}

/// Get the gas used, accounting for refunds
pub fn gas_used(spec: SpecId, spent: u64, refunded: u64) -> u64 {
    let refund_quotient = if SpecId::enabled(spec, SpecId::LONDON) { 5 } else { 2 };
    spent - (refunded).min(spent / refund_quotient)
}
