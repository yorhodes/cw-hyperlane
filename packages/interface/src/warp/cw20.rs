use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::Binary;

use crate::{core::mailbox, router};

use super::{TokenMode, TokenType};

#[cw_serde]
pub enum ReceiveMsg {
    // transfer to remote
    TransferRemote { dest_domain: u32, recipient: Binary },
}

#[cw_serde]
pub enum ExecuteMsg {
    Router(router::RouterMsg<Binary>),

    /// handle transfer remote
    Handle(mailbox::HandleMsg),

    // cw20 receiver
    Receive(cw20::Cw20ReceiveMsg),
}

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(router::DomainsResponse)]
    Domains {},

    #[returns(router::RouteResponse<Binary>)]
    Router { domain: u32 },

    #[returns(TokenTypeResponse)]
    TokenType {},

    #[returns(TokenModeResponse)]
    TokenMode {},
}

#[cw_serde]
pub struct TokenTypeResponse {
    #[serde(rename = "type")]
    pub typ: TokenType,
}

#[cw_serde]
pub struct TokenModeResponse {
    pub mode: TokenMode,
}
