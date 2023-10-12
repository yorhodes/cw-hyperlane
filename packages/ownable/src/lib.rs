#[cfg(test)]
mod test;

use cosmwasm_std::{
    to_binary, Addr, CustomQuery, Deps, DepsMut, Env, Event, MessageInfo, QueryResponse, Response,
    StdError, StdResult, Storage,
};
use cw_storage_plus::Item;
use hpl_interface::ownable::{OwnableMsg, OwnableQueryMsg, OwnerResponse, PendingOwnerResponse};

const OWNER_KEY: &str = "owner";
const OWNER: Item<Addr> = Item::new(OWNER_KEY);

const PENDING_OWNER_KEY: &str = "pending_owner";
const PENDING_OWNER: Item<Addr> = Item::new(PENDING_OWNER_KEY);

fn new_event(name: &str) -> Event {
    Event::new(format!("hpl_ownable::{}", name))
}

pub fn initialize(storage: &mut dyn Storage, owner: &Addr) -> StdResult<()> {
    OWNER.save(storage, owner)?;

    Ok(())
}

pub fn handle<C: CustomQuery>(
    deps: DepsMut<'_, C>,
    _env: Env,
    info: MessageInfo,
    msg: OwnableMsg,
) -> StdResult<Response> {
    use OwnableMsg::*;

    match msg {
        InitOwnershipTransfer { next_owner } => {
            Ok(Response::new().add_event(init_ownership_transfer(
                deps.storage,
                &info.sender,
                &deps.api.addr_validate(&next_owner)?,
            )?))
        }
        RevokeOwnershipTransfer {} => {
            Ok(Response::new().add_event(revoke_ownership_transfer(deps.storage, &info.sender)?))
        }
        ClaimOwnership {} => {
            Ok(Response::new().add_event(claim_ownership(deps.storage, &info.sender)?))
        }
    }
}

pub fn init_ownership_transfer(
    storage: &mut dyn Storage,
    sender: &Addr,
    next_owner: &Addr,
) -> StdResult<Event> {
    if sender != OWNER.load(storage)? {
        return Err(StdError::generic_err("unauthorized"));
    }
    if PENDING_OWNER.exists(storage) {
        return Err(StdError::generic_err("ownership is transferring"));
    }

    PENDING_OWNER.save(storage, next_owner)?;

    Ok(new_event("init")
        .add_attribute("owner", sender)
        .add_attribute("next_owner", next_owner))
}

pub fn revoke_ownership_transfer(storage: &mut dyn Storage, sender: &Addr) -> StdResult<Event> {
    if sender != OWNER.load(storage)? {
        return Err(StdError::generic_err("unauthorized"));
    }
    if !PENDING_OWNER.exists(storage) {
        return Err(StdError::generic_err("ownership is not transferring"));
    }

    PENDING_OWNER.remove(storage);

    Ok(new_event("revoke").add_attribute("owner", sender))
}

pub fn claim_ownership(storage: &mut dyn Storage, sender: &Addr) -> StdResult<Event> {
    if !PENDING_OWNER.exists(storage) {
        return Err(StdError::generic_err("ownership is not transferring"));
    }
    if sender != PENDING_OWNER.load(storage)? {
        return Err(StdError::generic_err("unauthorized"));
    }

    OWNER.save(storage, sender)?;
    PENDING_OWNER.remove(storage);

    Ok(new_event("claim").add_attribute("owner", sender))
}

pub fn handle_query<C: CustomQuery>(
    deps: Deps<'_, C>,
    _env: Env,
    msg: OwnableQueryMsg,
) -> StdResult<QueryResponse> {
    match msg {
        OwnableQueryMsg::GetOwner {} => to_binary(&OwnerResponse {
            owner: get_owner(deps.storage)?,
        }),
        OwnableQueryMsg::GetPendingOwner {} => to_binary(&PendingOwnerResponse {
            pending_owner: get_pending_owner(deps.storage)?,
        }),
    }
}

pub fn get_owner(storage: &dyn Storage) -> StdResult<Addr> {
    let owner = OWNER.load(storage)?;

    Ok(owner)
}

pub fn get_pending_owner(storage: &dyn Storage) -> StdResult<Option<Addr>> {
    let pending_owner = PENDING_OWNER.may_load(storage)?;

    Ok(pending_owner)
}
