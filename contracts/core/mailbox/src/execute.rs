use cosmwasm_std::{
    ensure, ensure_eq, to_binary, wasm_execute, DepsMut, HexBinary, MessageInfo, Response,
};
use hpl_interface::{
    core::{
        mailbox::{DispatchMsg, DispatchResponse},
        HandleMsg,
    },
    hook::post_dispatch,
    ism,
    types::Message,
};

use hpl_ownable::get_owner;

use crate::{
    event::{
        emit_default_hook_set, emit_default_ism_set, emit_dispatch, emit_dispatch_id, emit_process,
        emit_process_id,
    },
    state::{Delivery, CONFIG, DELIVERIES, LATEST_DISPATCHED_ID, NONCE},
    ContractError, MAILBOX_VERSION,
};

pub fn set_default_ism(
    deps: DepsMut,
    info: MessageInfo,
    new_default_ism: String,
) -> Result<Response, ContractError> {
    ensure_eq!(
        get_owner(deps.storage)?,
        info.sender,
        ContractError::Unauthorized {}
    );

    let new_default_ism = deps.api.addr_validate(&new_default_ism)?;
    let event = emit_default_ism_set(info.sender, new_default_ism.clone());

    CONFIG.update(deps.storage, |mut config| -> Result<_, ContractError> {
        config.default_ism = Some(new_default_ism);

        Ok(config)
    })?;

    Ok(Response::new().add_event(event))
}

pub fn set_default_hook(
    deps: DepsMut,
    info: MessageInfo,
    new_default_hook: String,
) -> Result<Response, ContractError> {
    ensure_eq!(
        get_owner(deps.storage)?,
        info.sender,
        ContractError::Unauthorized {}
    );

    let new_default_hook = deps.api.addr_validate(&new_default_hook)?;
    let event = emit_default_hook_set(info.sender, new_default_hook.clone());

    CONFIG.update(deps.storage, |mut config| -> Result<_, ContractError> {
        config.default_hook = Some(new_default_hook);

        Ok(config)
    })?;

    Ok(Response::new().add_event(event))
}

pub fn dispatch(
    deps: DepsMut,
    info: MessageInfo,
    dispatch_msg: DispatchMsg,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    let nonce = NONCE.load(deps.storage)?;

    ensure!(
        dispatch_msg.recipient_addr.len() <= 32,
        ContractError::InvalidAddressLength {
            len: dispatch_msg.recipient_addr.len()
        }
    );

    // interaction
    let hook = dispatch_msg
        .get_hook_addr(deps.api, config.default_hook)?
        .expect("default_hook not set");
    let hook_metadata = dispatch_msg.metadata.clone();

    let msg = dispatch_msg.to_msg(MAILBOX_VERSION, nonce, config.local_domain, &info.sender)?;

    let message_id = msg.id();

    // effects
    NONCE.save(deps.storage, &(nonce + 1))?;
    LATEST_DISPATCHED_ID.save(deps.storage, &message_id.to_vec())?;

    // make message
    let post_dispatch_msg = post_dispatch(
        hook,
        hook_metadata.unwrap_or_default(),
        msg.clone(),
        Some(info.funds),
    )?;

    Ok(Response::new()
        .add_event(emit_dispatch_id(message_id.clone()))
        .add_event(emit_dispatch(msg))
        .set_data(to_binary(&DispatchResponse { message_id })?)
        .add_message(post_dispatch_msg))
}

pub fn process(
    deps: DepsMut,
    info: MessageInfo,
    metadata: HexBinary,
    message: HexBinary,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;

    let decoded_msg: Message = message.clone().into();
    let recipient = decoded_msg.recipient_addr(&config.hrp)?;

    ensure_eq!(
        decoded_msg.version,
        MAILBOX_VERSION,
        ContractError::InvalidMessageVersion {
            version: decoded_msg.version
        }
    );
    ensure_eq!(
        decoded_msg.dest_domain,
        config.local_domain,
        ContractError::InvalidDestinationDomain {
            domain: decoded_msg.dest_domain
        }
    );

    let id = decoded_msg.id();
    let ism = ism::recipient(&deps.querier, &recipient)?.unwrap_or(config.get_default_ism());

    ensure!(
        !DELIVERIES.has(deps.storage, id.to_vec()),
        ContractError::AlreadyDeliveredMessage {}
    );

    DELIVERIES.save(
        deps.storage,
        id.to_vec(),
        &Delivery {
            sender: info.sender,
        },
    )?;

    ensure!(
        ism::verify(&deps.querier, ism, metadata, message)?,
        ContractError::VerifyFailed {}
    );

    let handle_msg = wasm_execute(
        recipient,
        &HandleMsg {
            origin: decoded_msg.origin_domain,
            sender: decoded_msg.sender.clone(),
            body: decoded_msg.body,
        }
        .wrap(),
        vec![],
    )?;

    Ok(Response::new().add_message(handle_msg).add_events(vec![
        emit_process_id(id),
        emit_process(
            config.local_domain,
            decoded_msg.sender,
            decoded_msg.recipient,
        ),
    ]))
}

#[cfg(test)]
mod tests {
    use cosmwasm_std::{
        testing::{mock_dependencies, mock_info},
        Addr, QuerierResult, SystemResult, WasmQuery,
    };

    use hpl_interface::types::bech32_encode;
    use rstest::rstest;

    use super::*;

    use crate::state::Config;

    const OWNER: &str = "owner";
    const NOT_OWNER: &str = "not_owner";

    const LOCAL_DOMAIN: u32 = 26657;
    const DEST_DOMAIN: u32 = 11155111;

    fn addr(v: &str) -> Addr {
        Addr::unchecked(v)
    }

    fn gen_bz(len: usize) -> HexBinary {
        let bz: Vec<_> = (0..len).map(|_| rand::random::<u8>()).collect();
        bz.into()
    }

    #[rstest]
    #[case(addr(OWNER), addr("default_ism"), Ok(()))]
    #[case(addr(NOT_OWNER), addr("default_ism"), Err(ContractError::Unauthorized{}))]
    fn test_set_default_ism(
        #[case] sender: Addr,
        #[case] new_default_ism: Addr,
        #[case] expected: Result<(), ContractError>,
    ) {
        let expected = expected.map(|_| {
            Response::new().add_event(emit_default_ism_set(
                sender.clone(),
                new_default_ism.clone(),
            ))
        });

        let mut deps = mock_dependencies();

        CONFIG
            .save(deps.as_mut().storage, &Default::default())
            .unwrap();

        hpl_ownable::initialize(deps.as_mut().storage, &addr(OWNER)).unwrap();

        assert_eq!(
            expected,
            set_default_ism(
                deps.as_mut(),
                mock_info(sender.as_str(), &[]),
                new_default_ism.to_string()
            )
        );
    }

    #[rstest]
    #[case(addr(OWNER), addr("default_hook"), Ok(()))]
    #[case(addr(NOT_OWNER), addr("default_hook"), Err(ContractError::Unauthorized{}))]
    fn test_set_default_hook(
        #[case] sender: Addr,
        #[case] new_default_hook: Addr,
        #[case] expected: Result<(), ContractError>,
    ) {
        let expected = expected.map(|_| {
            Response::new().add_event(emit_default_hook_set(
                sender.clone(),
                new_default_hook.clone(),
            ))
        });

        let mut deps = mock_dependencies();

        CONFIG
            .save(deps.as_mut().storage, &Default::default())
            .unwrap();

        hpl_ownable::initialize(deps.as_mut().storage, &addr(OWNER)).unwrap();

        assert_eq!(
            expected,
            set_default_hook(
                deps.as_mut(),
                mock_info(sender.as_str(), &[]),
                new_default_hook.to_string()
            )
        );
    }

    #[rstest]
    #[case(DEST_DOMAIN, gen_bz(20), gen_bz(32), Ok(()))]
    #[case(DEST_DOMAIN, gen_bz(20), gen_bz(33), Err(ContractError::InvalidAddressLength { len: 33 }))]
    fn test_dispatch(
        #[values("osmo", "neutron")] hrp: &str,
        #[case] dest_domain: u32,
        #[case] sender: HexBinary,
        #[case] recipient_addr: HexBinary,
        #[case] expected: Result<(), ContractError>,
    ) {
        let sender = bech32_encode(hrp, sender.as_slice()).unwrap();
        let msg_body = gen_bz(123);

        let mut deps = mock_dependencies();

        CONFIG
            .save(
                deps.as_mut().storage,
                &Config::new(hrp, LOCAL_DOMAIN)
                    .with_hook(addr("default_hook"))
                    .with_ism(addr("default_ism")),
            )
            .unwrap();
        NONCE.save(deps.as_mut().storage, &0u32).unwrap();

        hpl_ownable::initialize(deps.as_mut().storage, &addr(OWNER)).unwrap();

        let dispatch_msg = DispatchMsg::new(dest_domain, recipient_addr, msg_body);
        let msg = dispatch_msg
            .clone()
            .to_msg(
                MAILBOX_VERSION,
                NONCE.load(deps.as_ref().storage).unwrap(),
                LOCAL_DOMAIN,
                &sender,
            )
            .unwrap();

        let res = dispatch(deps.as_mut(), mock_info(sender.as_str(), &[]), dispatch_msg);
        assert_eq!(res.map(|_| ()), expected);

        if expected.is_ok() {
            assert_eq!(NONCE.load(deps.as_ref().storage).unwrap(), 1u32);
            assert_eq!(
                LATEST_DISPATCHED_ID.load(deps.as_ref().storage).unwrap(),
                msg.id().to_vec()
            );
        }
    }

    fn test_process_query_handler(query: &WasmQuery) -> QuerierResult {
        match query {
            WasmQuery::Smart { contract_addr, msg } => {
                if let Ok(req) = cosmwasm_std::from_binary::<ism::ISMSpecifierQueryMsg>(msg) {
                    match req {
                        ism::ISMSpecifierQueryMsg::InterchainSecurityModule() => {
                            return SystemResult::Ok(
                                cosmwasm_std::to_binary(&ism::InterchainSecurityModuleResponse {
                                    ism: Some(addr("default_ism")),
                                })
                                .into(),
                            );
                        }
                    }
                }

                if let Ok(req) = cosmwasm_std::from_binary::<ism::ISMQueryMsg>(msg) {
                    assert_eq!(contract_addr, &addr("default_ism"));

                    match req {
                        ism::ISMQueryMsg::Verify { metadata, .. } => {
                            return SystemResult::Ok(
                                cosmwasm_std::to_binary(&ism::VerifyResponse {
                                    verified: metadata[0] == 1,
                                })
                                .into(),
                            );
                        }
                        _ => unreachable!("not in test coverage"),
                    }
                }

                unreachable!("not in test coverage")
            }
            _ => unimplemented!("only for smart query"),
        }
    }

    #[rstest]
    #[case(MAILBOX_VERSION, LOCAL_DOMAIN, gen_bz(32), false, true)]
    #[should_panic(expected = "invalid message version: 99")]
    #[case(99, LOCAL_DOMAIN, gen_bz(32), false, true)]
    #[should_panic(expected = "message already delivered")]
    #[case(MAILBOX_VERSION, LOCAL_DOMAIN, gen_bz(32), true, true)]
    #[should_panic(expected = "invalid destination domain: 11155111")]
    #[case(MAILBOX_VERSION, DEST_DOMAIN, gen_bz(32), false, true)]
    #[should_panic(expected = "ism verify failed")]
    #[case(MAILBOX_VERSION, LOCAL_DOMAIN, gen_bz(32), false, false)]
    fn test_process_revised(
        #[values("osmo", "neutron")] hrp: &str,
        #[case] version: u8,
        #[case] dest_domain: u32,
        #[case] recipient_addr: HexBinary,
        #[case] duplicate: bool,
        #[case] verified: bool,
    ) {
        let sender = gen_bz(32);
        let sender_addr = bech32_encode(hrp, &sender).unwrap();
        let msg_body = gen_bz(123);

        let mut deps = mock_dependencies();

        deps.querier.update_wasm(test_process_query_handler);

        CONFIG
            .save(
                deps.as_mut().storage,
                &Config::new(hrp, LOCAL_DOMAIN)
                    .with_hook(addr("default_hook"))
                    .with_ism(addr("default_ism")),
            )
            .unwrap();

        let msg = Message {
            version,
            nonce: 123,
            origin_domain: DEST_DOMAIN,
            sender,
            dest_domain,
            recipient: recipient_addr,
            body: msg_body,
        };
        let msg_id = msg.id();

        if duplicate {
            DELIVERIES
                .save(
                    deps.as_mut().storage,
                    msg.id().to_vec(),
                    &Delivery {
                        sender: sender_addr.clone(),
                    },
                )
                .unwrap();
        }

        let _res = process(
            deps.as_mut(),
            mock_info(sender_addr.as_str(), &[]),
            vec![verified.into()].into(),
            msg.into(),
        )
        .map_err(|v| v.to_string())
        .unwrap();

        let delivery = DELIVERIES
            .load(deps.as_ref().storage, msg_id.to_vec())
            .unwrap();
        assert_eq!(delivery.sender, sender_addr);
    }
}
