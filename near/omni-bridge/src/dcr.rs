use crate::{
    ext_token, Contract, ContractExt, Role, FT_TRANSFER_CALL_GAS, ONE_YOCTO,
};
use near_plugins::{access_control_any, pause, AccessControllable, Pausable};
use near_sdk::json_types::{U128, U64};
use near_sdk::{
    env, near, require, serde_json, AccountId, Gas, Promise, PromiseError, PromiseOrValue,
};
use omni_types::dcr::DcrTokenReceiverMessage;
use omni_types::{ChainKind, Fee, TransferId, TransferMessage};

/// Gas allocated for the callback after calling the DCR connector
const SUBMIT_TRANSFER_TO_DCR_CONNECTOR_CALLBACK_GAS: Gas = Gas::from_tgas(5);

/// Extra metadata embedded inside TransferMessage.msg for UTXO chains (specific to DCR)
#[near(serializers = [json])]
#[derive(Debug, PartialEq)]
enum DcrUtxoChainMsg {
    /// Maximum fee rate (atoms per kB)
    MaxFeeRate(U64),
}

#[near]
impl Contract {
    /// Submits a transfer to the DCR connector.
    ///
    /// This is **specific to Decred** and does NOT affect BTC logic.
    /// Frontend / relayer should call this function when the destination chain is DCR.
    #[payable]
    #[pause(except(roles(Role::DAO, Role::UnrestrictedRelayer)))]
    pub fn submit_transfer_to_dcr_connector(
        &mut self,
        transfer_id: TransferId,
        msg: String,
        fee_recipient: Option<AccountId>,
        fee: &Option<Fee>,
    ) -> Promise {
        let transfer = self.get_transfer_message_storage(transfer_id);

        // Parse incoming message as DCR-specific receiver message
        let message =
            serde_json::from_str::<DcrTokenReceiverMessage>(&msg).expect("INVALID DCR MSG");

        // Actual transferable amount = amount - token fee
        let amount = U128(transfer.message.amount.0 - transfer.message.fee.fee.0);

        // Ensure destination is a valid UTXO address (DCR address)
        if let Some(dcr_address) = transfer.message.recipient.get_utxo_address() {
            if let DcrTokenReceiverMessage::Withdraw {
                target_dcr_address,
                input: _,
                output: _,
                max_fee_rate,
            } = message
            {
                // The address inside the TransferMessage must match the message payload
                require!(
                    dcr_address == target_dcr_address,
                    "Incorrect target address"
                );

                // If TransferMessage.msg has embedded metadata, cross-check max fee rate
                if !transfer.message.msg.is_empty() {
                    let utxo_chain_extra_info: DcrUtxoChainMsg =
                        serde_json::from_str(&transfer.message.msg)
                            .expect("Invalid Transfer MSG for DCR UTXO chain");

                    let DcrUtxoChainMsg::MaxFeeRate(max_fee_rate_from_msg) =
                        utxo_chain_extra_info;

                    require!(
                        max_fee_rate
                            .expect("max_fee_rate is missing")
                            .0
                            == max_fee_rate_from_msg.0.into(),
                        "Invalid max fee rate"
                    );
                }
            } else {
                env::panic_str("Invalid DCR message type");
            }
        } else {
            env::panic_str("Invalid destination chain for DCR");
        }

        // If fee is explicitly provided, validate it
        if let Some(fee) = &fee {
            require!(&transfer.message.fee == fee, "Invalid fee");
        }

        // Destination chain must be DCR
        let chain_kind = transfer.message.get_destination_chain();
        require!(
            chain_kind == ChainKind::Dcr,
            "submit_transfer_to_dcr_connector can only be used for Decred"
        );

        // wDCR token (NEP-141 wrapper) must match the transfer token
        let dcr_token_id = self.get_utxo_chain_token(chain_kind);
        require!(
            self.get_token_id(&transfer.message.token) == dcr_token_id,
            "Only the native token of this UTXO chain can be transferred."
        );

        // Remove the transfer from storage (it will be restored if callback fails)
        self.remove_transfer_message(transfer_id);

        // Fee recipient defaults to predecessor if not specified
        let fee_recipient = fee_recipient.unwrap_or(env::predecessor_account_id());

        // Forward the transfer to the DCR connector using ft_transfer_call
        ext_token::ext(dcr_token_id)
            .with_attached_deposit(ONE_YOCTO)
            .with_static_gas(FT_TRANSFER_CALL_GAS)
            .ft_transfer_call(self.get_utxo_chain_connector(chain_kind), amount, None, msg)
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(SUBMIT_TRANSFER_TO_DCR_CONNECTOR_CALLBACK_GAS)
                    .submit_transfer_to_dcr_connector_callback(
                        transfer.message,
                        transfer.owner,
                        fee_recipient,
                    ),
            )
    }

    /// Callback after calling ft_transfer_call to the DCR connector.
    ///
    /// - If successful (result > 0): send the fee.
    /// - If failed: restore the transfer so it can be retried.
    #[private]
    pub fn submit_transfer_to_dcr_connector_callback(
        &mut self,
        transfer_msg: TransferMessage,
        transfer_owner: AccountId,
        fee_recipient: AccountId,
        #[callback_result] call_result: &Result<U128, PromiseError>,
    ) -> PromiseOrValue<()> {
        if matches!(call_result, Ok(result) if result.0 > 0) {
            let token_fee = transfer_msg.fee.fee.0;
            self.send_fee_internal(&transfer_msg, fee_recipient, token_fee)
        } else {
            self.insert_raw_transfer(transfer_msg, transfer_owner);
            PromiseOrValue::Value(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deserialize_dcr_utxo_chain_msg() {
        let serialized_msg = r#"{"MaxFeeRate":"12345"}"#;
        let deserialized: DcrUtxoChainMsg = serde_json::from_str(serialized_msg).unwrap();
        let original = DcrUtxoChainMsg::MaxFeeRate(12345.into());
        assert_eq!(original, deserialized);
    }
}
