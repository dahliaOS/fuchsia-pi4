// Copyright 2021 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

mod link_state;

use {
    crate::{
        capabilities::{intersect_with_ap_as_client, ClientCapabilities},
        client::{
            bss::ClientConfig,
            capabilities::derive_join_channel_and_capabilities,
            event::{self, Event},
            info::{DisconnectCause, DisconnectInfo, DisconnectMlmeEventName, DisconnectSource},
            internal::Context,
            protection::{build_protection_ie, Protection, ProtectionIe},
            report_connect_finished, AssociationFailure, ConnectFailure, ConnectResult,
            EstablishRsnaFailure, EstablishRsnaFailureReason, Status,
        },
        phy_selection::derive_phy_cbw,
        responder::Responder,
        sink::MlmeSink,
        timer::EventId,
        MlmeRequest,
    },
    anyhow::bail,
    fidl_fuchsia_wlan_ieee80211 as fidl_ieee80211, fidl_fuchsia_wlan_internal as fidl_internal,
    fidl_fuchsia_wlan_mlme::{self as fidl_mlme, MlmeEvent},
    fidl_fuchsia_wlan_sme as fidl_sme,
    fuchsia_inspect_contrib::{inspect_log, log::InspectBytes},
    fuchsia_zircon as zx,
    link_state::LinkState,
    log::{error, info, warn},
    static_assertions::assert_eq_size,
    std::convert::TryInto,
    wep_deprecated,
    wlan_common::{
        bss::BssDescription,
        channel::Channel,
        format::{MacFmt as _, SsidFmt as _},
        ie::{self, rsn::cipher},
        mac::Bssid,
        RadioConfig,
    },
    wlan_rsn::{
        auth,
        rsna::{AuthStatus, SecAssocUpdate, UpdateSink},
    },
    wlan_statemachine::*,
    zerocopy::AsBytes,
};
const DEFAULT_JOIN_FAILURE_TIMEOUT: u32 = 20; // beacon intervals
const DEFAULT_AUTH_FAILURE_TIMEOUT: u32 = 20; // beacon intervals

const IDLE_STATE: &str = "IdleState";
const JOINING_STATE: &str = "JoiningState";
const AUTHENTICATING_STATE: &str = "AuthenticatingState";
const ASSOCIATING_STATE: &str = "AssociatingState";
const RSNA_STATE: &str = "EstablishingRsnaState";
const LINK_UP_STATE: &str = "LinkUpState";

#[derive(Debug)]
pub struct ConnectCommand {
    pub bss: Box<BssDescription>,
    pub responder: Option<Responder<ConnectResult>>,
    pub protection: Protection,
    pub radio_cfg: RadioConfig,
}

#[derive(Debug)]
pub struct Idle {
    cfg: ClientConfig,
}

#[derive(Debug)]
pub struct Joining {
    cfg: ClientConfig,
    cmd: ConnectCommand,
    chan: Channel,
    cap: Option<ClientCapabilities>,
    protection_ie: Option<ProtectionIe>,
}

#[derive(Debug)]
pub struct Authenticating {
    cfg: ClientConfig,
    cmd: ConnectCommand,
    chan: Channel,
    cap: Option<ClientCapabilities>,
    protection_ie: Option<ProtectionIe>,
}

#[derive(Debug)]
pub struct Associating {
    cfg: ClientConfig,
    cmd: ConnectCommand,
    chan: Channel,
    cap: Option<ClientCapabilities>,
    protection_ie: Option<ProtectionIe>,
}

#[derive(Debug)]
pub struct Associated {
    cfg: ClientConfig,
    responder: Option<Responder<ConnectResult>>,
    bss: Box<BssDescription>,
    auth_method: Option<auth::MethodName>,
    last_rssi: i8,
    last_snr: i8,
    last_signal_report_time: zx::Time,
    link_state: LinkState,
    radio_cfg: RadioConfig,
    chan: Channel,
    cap: Option<ClientCapabilities>,
    protection_ie: Option<ProtectionIe>,
    wmm_param: Option<ie::WmmParam>,
    last_channel_switch_time: Option<zx::Time>,
}

statemachine!(
    #[derive(Debug)]
    pub enum ClientState,
    () => Idle,
    Idle => Joining,
    Joining => [Authenticating, Idle],
    Authenticating => [Associating, Idle],
    Associating => [Associated, Idle],
    // We transition back to Associating on a disassociation ind.
    Associated => [Idle, Associating],
);

/// Context surrounding the state change, for Inspect logging
pub enum StateChangeContext {
    Disconnect { msg: String, disconnect_source: DisconnectSource },
    Msg(String),
}

trait StateChangeContextExt {
    fn set_msg(&mut self, msg: String);
}

impl StateChangeContextExt for Option<StateChangeContext> {
    fn set_msg(&mut self, msg: String) {
        match self {
            Some(ctx) => match ctx {
                StateChangeContext::Disconnect { msg: ref mut inner, .. } => *inner = msg,
                StateChangeContext::Msg(inner) => *inner = msg,
            },
            None => {
                self.replace(StateChangeContext::Msg(msg));
            }
        }
    }
}

impl Joining {
    fn on_join_conf(
        self,
        conf: fidl_mlme::JoinConfirm,
        state_change_ctx: &mut Option<StateChangeContext>,
        context: &mut Context,
    ) -> Result<Authenticating, Idle> {
        match conf.result_code {
            fidl_mlme::JoinResultCode::Success => {
                context.info.report_auth_started();
                let (auth_type, sae_password) = match &self.cmd.protection {
                    Protection::Rsna(rsna) => match rsna.supplicant.get_auth_cfg() {
                        auth::Config::Sae { .. } => (fidl_mlme::AuthenticationTypes::Sae, None),
                        auth::Config::DriverSae { password } => {
                            (fidl_mlme::AuthenticationTypes::Sae, Some(password.clone()))
                        }
                        auth::Config::ComputedPsk(_) => {
                            (fidl_mlme::AuthenticationTypes::OpenSystem, None)
                        }
                    },
                    Protection::Wep(ref key) => {
                        install_wep_key(context, self.cmd.bss.bssid.clone(), key);
                        (fidl_mlme::AuthenticationTypes::SharedKey, None)
                    }
                    _ => (fidl_mlme::AuthenticationTypes::OpenSystem, None),
                };

                context.mlme_sink.send(MlmeRequest::Authenticate(fidl_mlme::AuthenticateRequest {
                    peer_sta_address: self.cmd.bss.bssid.clone(),
                    auth_type,
                    auth_failure_timeout: DEFAULT_AUTH_FAILURE_TIMEOUT,
                    sae_password,
                }));

                state_change_ctx.set_msg("successful join".to_string());
                Ok(Authenticating {
                    cfg: self.cfg,
                    cmd: self.cmd,
                    chan: self.chan,
                    cap: self.cap,
                    protection_ie: self.protection_ie,
                })
            }
            other => {
                error!("Join request failed with result code {:?}", other);
                report_connect_finished(
                    self.cmd.responder,
                    context,
                    ConnectResult::Failed(ConnectFailure::JoinFailure(other)),
                );
                state_change_ctx.set_msg(format!("join failed; result code: {:?}", other));
                Err(Idle { cfg: self.cfg })
            }
        }
    }
}

impl Authenticating {
    fn on_authenticate_conf(
        self,
        conf: fidl_mlme::AuthenticateConfirm,
        state_change_ctx: &mut Option<StateChangeContext>,
        context: &mut Context,
    ) -> Result<Associating, Idle> {
        match conf.result_code {
            fidl_mlme::AuthenticateResultCode::Success => {
                context.info.report_assoc_started();
                send_mlme_assoc_req(
                    Bssid(self.cmd.bss.bssid.clone()),
                    self.cap.as_ref(),
                    &self.protection_ie,
                    &context.mlme_sink,
                );
                state_change_ctx.set_msg("successful authentication".to_string());
                Ok(Associating {
                    cfg: self.cfg,
                    cmd: self.cmd,
                    chan: self.chan,
                    cap: self.cap,

                    protection_ie: self.protection_ie,
                })
            }
            other => {
                error!("Authenticate request failed with result code {:?}", other);
                report_connect_finished(
                    self.cmd.responder,
                    context,
                    ConnectResult::Failed(ConnectFailure::AuthenticationFailure(other)),
                );
                state_change_ctx.set_msg(format!("auth failed; result code: {:?}", other));
                Err(Idle { cfg: self.cfg })
            }
        }
    }

    fn on_deauthenticate_ind(
        self,
        ind: fidl_mlme::DeauthenticateIndication,
        state_change_ctx: &mut Option<StateChangeContext>,
        context: &mut Context,
    ) -> Idle {
        error!(
            "authentication request failed due to spurious deauthentication: {:?}",
            ind.reason_code
        );
        report_connect_finished(
            self.cmd.responder,
            context,
            ConnectResult::Failed(ConnectFailure::AuthenticationFailure(
                fidl_mlme::AuthenticateResultCode::Refused,
            )),
        );
        state_change_ctx.set_msg(format!(
            "received DeauthenticateInd msg; reason code: {:?}, locally_initiated: {:?}",
            ind.reason_code, ind.locally_initiated,
        ));
        Idle { cfg: self.cfg }
    }

    // Sae management functions

    fn on_pmk_available(&mut self, pmk: fidl_mlme::PmkInfo) -> Result<(), anyhow::Error> {
        let supplicant = match &mut self.cmd.protection {
            Protection::Rsna(rsna) => &mut rsna.supplicant,
            _ => bail!("Unexpected SAE handshake indication"),
        };

        let mut updates = UpdateSink::default();
        supplicant.on_pmk_available(&mut updates, &pmk.pmk[..], &pmk.pmkid[..])?;
        // We don't do anything with these updates right now.
        Ok(())
    }

    fn on_sae_handshake_ind(
        &mut self,
        ind: fidl_mlme::SaeHandshakeIndication,
        context: &mut Context,
    ) -> Result<(), anyhow::Error> {
        process_sae_handshake_ind(&mut self.cmd.protection, ind, context)
    }

    fn on_sae_frame_rx(
        &mut self,
        frame: fidl_mlme::SaeFrame,
        context: &mut Context,
    ) -> Result<(), anyhow::Error> {
        process_sae_frame_rx(&mut self.cmd.protection, frame, context)
    }

    fn handle_timeout(
        mut self,
        _event_id: EventId,
        event: Event,
        state_change_ctx: &mut Option<StateChangeContext>,
        context: &mut Context,
    ) -> Result<Self, Idle> {
        match process_sae_timeout(&mut self.cmd.protection, self.cmd.bss.bssid, event, context) {
            Ok(()) => Ok(self),
            Err(e) => {
                // An error in handling a timeout means that we may have no way to abort a
                // failed handshake. Drop to idle.
                state_change_ctx.set_msg(format!("failed to handle SAE timeout: {:?}", e));
                return Err(Idle { cfg: self.cfg });
            }
        }
    }
}

impl Associating {
    fn on_associate_conf(
        self,
        conf: fidl_mlme::AssociateConfirm,
        state_change_ctx: &mut Option<StateChangeContext>,
        context: &mut Context,
    ) -> Result<Associated, Idle> {
        let auth_method = self.cmd.protection.get_rsn_auth_method();
        let wmm_param =
            conf.wmm_param.as_ref().and_then(|p| match ie::parse_wmm_param(&p.bytes[..]) {
                Ok(param) => Some(*param),
                Err(e) => {
                    warn!(
                        "Fail parsing assoc conf WMM param. Bytes: {:?}. Error: {}",
                        &p.bytes[..],
                        e
                    );
                    None
                }
            });
        let link_state =
            match conf.result_code {
                fidl_mlme::AssociateResultCode::Success => {
                    context.info.report_assoc_success(context.att_id);
                    if let Some(cap) = self.cap.as_ref() {
                        let negotiated_cap = intersect_with_ap_as_client(cap, &conf.into());
                        // TODO(eyw): Enable this check once we switch to Rust MLME which populates
                        // associate confirm with IEs.
                        if negotiated_cap.rates.is_empty() {
                            // This is unlikely to happen with any spec-compliant AP. In case the
                            // user somehow decided to connect to a malicious AP, reject and reset.
                            error!(
                                "Associate terminated because AP's capabilities in association \
                                 response is different from beacon"
                            );
                            report_connect_finished(
                            self.cmd.responder,
                            context,
                            ConnectResult::Failed(AssociationFailure{
                                bss_protection: self.cmd.bss.protection(),
                                code: fidl_mlme::AssociateResultCode::RefusedCapabilitiesMismatch,
                            }.into()),
                        );
                            state_change_ctx.set_msg(format!(
                                "failed associating; AP's capabilites changed between beacon and\
                                 association response"
                            ));
                            return Err(Idle { cfg: self.cfg });
                        }
                        context.mlme_sink.send(MlmeRequest::FinalizeAssociation(
                            negotiated_cap.to_fidl_negotiated_capabilities(&self.chan),
                        ))
                    }

                    match LinkState::new(self.cmd.protection, context) {
                        Ok(link_state) => link_state,
                        Err(failure_reason) => {
                            state_change_ctx.set_msg(format!("failed to initialized LinkState"));
                            send_deauthenticate_request(&self.cmd.bss, &context.mlme_sink);
                            report_connect_finished(
                                self.cmd.responder,
                                context,
                                EstablishRsnaFailure { auth_method, reason: failure_reason }.into(),
                            );
                            return Err(Idle { cfg: self.cfg });
                        }
                    }
                }
                other => {
                    error!("Associate request failed with result code {:?}", other);
                    report_connect_finished(
                        self.cmd.responder,
                        context,
                        ConnectResult::Failed(
                            AssociationFailure {
                                bss_protection: self.cmd.bss.protection(),
                                code: other,
                            }
                            .into(),
                        ),
                    );
                    state_change_ctx
                        .set_msg(format!("failed associating; result code: {:?}", other));
                    return Err(Idle { cfg: self.cfg });
                }
            };
        state_change_ctx.set_msg("successful assoc".to_string());

        let mut responder = self.cmd.responder;
        if let LinkState::LinkUp(_) = link_state {
            report_connect_finished(responder.take(), context, ConnectResult::Success);
        }

        Ok(Associated {
            cfg: self.cfg,
            responder,
            auth_method,
            last_rssi: self.cmd.bss.rssi_dbm,
            last_snr: self.cmd.bss.snr_db,
            last_signal_report_time: now(),
            bss: self.cmd.bss,
            link_state,
            radio_cfg: self.cmd.radio_cfg,
            chan: self.chan,
            cap: self.cap,
            protection_ie: self.protection_ie,
            wmm_param,
            last_channel_switch_time: None,
        })
    }

    fn on_deauthenticate_ind(
        self,
        ind: fidl_mlme::DeauthenticateIndication,
        state_change_ctx: &mut Option<StateChangeContext>,
        context: &mut Context,
    ) -> Idle {
        error!(
            "association request failed due to spurious deauthentication: {:?}",
            ind.reason_code
        );
        report_connect_finished(
            self.cmd.responder,
            context,
            ConnectResult::Failed(
                AssociationFailure {
                    bss_protection: self.cmd.bss.protection(),
                    code: fidl_mlme::AssociateResultCode::RefusedReasonUnspecified,
                }
                .into(),
            ),
        );
        state_change_ctx.set_msg(format!(
            "received DeauthenticateInd msg; reason code: {:?}, locally_initiated: {:?}",
            ind.reason_code, ind.locally_initiated,
        ));
        Idle { cfg: self.cfg }
    }

    fn on_disassociate_ind(
        self,
        ind: fidl_mlme::DisassociateIndication,
        state_change_ctx: &mut Option<StateChangeContext>,
        context: &mut Context,
    ) -> Idle {
        error!("association request failed due to spurious disassociation: {:?}", ind.reason_code);
        report_connect_finished(
            self.cmd.responder,
            context,
            ConnectResult::Failed(
                AssociationFailure {
                    bss_protection: self.cmd.bss.protection(),
                    code: fidl_mlme::AssociateResultCode::RefusedReasonUnspecified,
                }
                .into(),
            ),
        );
        state_change_ctx.set_msg(format!(
            "received DisassociateInd msg; reason code: {:?}, locally_initiated: {:?}",
            ind.reason_code, ind.locally_initiated,
        ));
        Idle { cfg: self.cfg }
    }

    // Sae management functions

    fn on_sae_handshake_ind(
        &mut self,
        ind: fidl_mlme::SaeHandshakeIndication,
        context: &mut Context,
    ) -> Result<(), anyhow::Error> {
        process_sae_handshake_ind(&mut self.cmd.protection, ind, context)
    }

    fn on_sae_frame_rx(
        &mut self,
        frame: fidl_mlme::SaeFrame,
        context: &mut Context,
    ) -> Result<(), anyhow::Error> {
        process_sae_frame_rx(&mut self.cmd.protection, frame, context)
    }

    fn handle_timeout(
        mut self,
        _event_id: EventId,
        event: Event,
        state_change_ctx: &mut Option<StateChangeContext>,
        context: &mut Context,
    ) -> Result<Self, Idle> {
        match process_sae_timeout(&mut self.cmd.protection, self.cmd.bss.bssid, event, context) {
            Ok(()) => Ok(self),
            Err(e) => {
                // An error in handling a timeout means that we may have no way to abort a
                // failed handshake. Drop to idle.
                state_change_ctx.set_msg(format!("failed to handle SAE timeout: {:?}", e));
                return Err(Idle { cfg: self.cfg });
            }
        }
    }
}

impl Associated {
    fn on_disassociate_ind(
        self,
        ind: fidl_mlme::DisassociateIndication,
        state_change_ctx: &mut Option<StateChangeContext>,
        context: &mut Context,
    ) -> Associating {
        let (mut protection, connected_duration) = self.link_state.disconnect();

        let disconnect_reason = DisconnectCause {
            mlme_event_name: DisconnectMlmeEventName::DisassociateIndication,
            reason_code: ind.reason_code,
        };
        let disconnect_source = if ind.locally_initiated {
            DisconnectSource::Mlme(disconnect_reason)
        } else {
            DisconnectSource::Ap(disconnect_reason)
        };

        if let Some(duration) = connected_duration {
            let disconnect_info = DisconnectInfo {
                connected_duration: duration,
                last_rssi: self.last_rssi,
                last_snr: self.last_snr,
                bssid: self.bss.bssid,
                ssid: self.bss.ssid().to_vec(),
                protection: self.bss.protection(),
                wsc: self.bss.probe_resp_wsc(),
                channel: Channel::from_fidl(self.bss.chan),
                disconnect_source,
                time_since_channel_switch: self.last_channel_switch_time.map(|t| now() - t),
            };
            context.info.report_disconnect(disconnect_info);
        }
        let msg = format!(
            "received DisassociateInd msg; reason code {:?}",
            disconnect_source.reason_code()
        );
        state_change_ctx.replace(match connected_duration {
            Some(_) => StateChangeContext::Disconnect { msg, disconnect_source },
            None => StateChangeContext::Msg(msg),
        });

        // Client is disassociating. The ESS-SA must be kept alive but reset.
        if let Protection::Rsna(rsna) = &mut protection {
            // Reset the state of the ESS-SA and its replay counter to zero per IEEE 802.11-2016 12.7.2.
            rsna.supplicant.reset();
        }

        context.att_id += 1;
        let cmd = ConnectCommand {
            bss: self.bss,
            responder: self.responder,
            protection,
            radio_cfg: self.radio_cfg,
        };
        send_mlme_assoc_req(
            Bssid(cmd.bss.bssid.clone()),
            self.cap.as_ref(),
            &self.protection_ie,
            &context.mlme_sink,
        );
        Associating {
            cfg: self.cfg,
            cmd,
            chan: self.chan,
            cap: self.cap,
            protection_ie: self.protection_ie,
        }
    }

    fn on_deauthenticate_ind(
        self,
        ind: fidl_mlme::DeauthenticateIndication,
        state_change_ctx: &mut Option<StateChangeContext>,
        context: &mut Context,
    ) -> Idle {
        let (_, connected_duration) = self.link_state.disconnect();

        let disconnect_reason = DisconnectCause {
            mlme_event_name: DisconnectMlmeEventName::DeauthenticateIndication,
            reason_code: ind.reason_code,
        };
        let disconnect_source = if ind.locally_initiated {
            DisconnectSource::Mlme(disconnect_reason)
        } else {
            DisconnectSource::Ap(disconnect_reason)
        };

        match connected_duration {
            Some(duration) => {
                let disconnect_info = DisconnectInfo {
                    connected_duration: duration,
                    last_rssi: self.last_rssi,
                    last_snr: self.last_snr,
                    bssid: self.bss.bssid,
                    ssid: self.bss.ssid().to_vec(),
                    protection: self.bss.protection(),
                    wsc: self.bss.probe_resp_wsc(),
                    channel: Channel::from_fidl(self.bss.chan),
                    disconnect_source,
                    time_since_channel_switch: self.last_channel_switch_time.map(|t| now() - t),
                };
                context.info.report_disconnect(disconnect_info);
            }
            None => {
                let connect_result = EstablishRsnaFailure {
                    auth_method: self.auth_method,
                    reason: EstablishRsnaFailureReason::InternalError,
                }
                .into();
                report_connect_finished(self.responder, context, connect_result);
            }
        }

        state_change_ctx.replace(StateChangeContext::Disconnect {
            msg: format!(
                "received DeauthenticateInd msg; reason code {:?}",
                disconnect_source.reason_code()
            ),
            disconnect_source,
        });
        Idle { cfg: self.cfg }
    }

    fn process_link_state_update<U, H>(
        self,
        update: U,
        update_handler: H,
        context: &mut Context,
        state_change_ctx: &mut Option<StateChangeContext>,
    ) -> Result<Self, Idle>
    where
        H: Fn(
            LinkState,
            U,
            &BssDescription,
            &mut Option<StateChangeContext>,
            &mut Context,
        ) -> Result<LinkState, EstablishRsnaFailureReason>,
    {
        let link_state =
            match update_handler(self.link_state, update, &self.bss, state_change_ctx, context) {
                Ok(link_state) => link_state,
                Err(failure_reason) => {
                    report_connect_finished(
                        self.responder,
                        context,
                        EstablishRsnaFailure {
                            auth_method: self.auth_method,
                            reason: failure_reason,
                        }
                        .into(),
                    );
                    send_deauthenticate_request(&self.bss, &context.mlme_sink);
                    return Err(Idle { cfg: self.cfg });
                }
            };

        let mut responder = self.responder;
        if let LinkState::LinkUp(_) = link_state {
            context.info.report_rsna_established(context.att_id);
            report_connect_finished(responder.take(), context, ConnectResult::Success);
        }

        Ok(Self { link_state, responder, ..self })
    }

    fn on_eapol_ind(
        self,
        ind: fidl_mlme::EapolIndication,
        state_change_ctx: &mut Option<StateChangeContext>,
        context: &mut Context,
    ) -> Result<Self, Idle> {
        // Ignore unexpected EAPoL frames.
        if !self.bss.needs_eapol_exchange() {
            return Ok(self);
        }

        // Reject EAPoL frames from other BSS.
        if ind.src_addr != self.bss.bssid {
            let eapol_pdu = &ind.data[..];
            inspect_log!(context.inspect.rsn_events.lock(), {
                rx_eapol_frame: InspectBytes(&eapol_pdu),
                foreign_bssid: ind.src_addr.to_mac_str(),
                foreign_bssid_hash: context.inspect.hasher.hash_mac_addr(&ind.src_addr),
                current_bssid: self.bss.bssid.to_mac_str(),
                current_bssid_hash: context.inspect.hasher.hash_mac_addr(&self.bss.bssid),
                status: "rejected (foreign BSS)",
            });
            return Ok(self);
        }

        self.process_link_state_update(ind, LinkState::on_eapol_ind, context, state_change_ctx)
    }

    fn on_eapol_conf(
        self,
        resp: fidl_mlme::EapolConfirm,
        state_change_ctx: &mut Option<StateChangeContext>,
        context: &mut Context,
    ) -> Result<Self, Idle> {
        self.process_link_state_update(resp, LinkState::on_eapol_conf, context, state_change_ctx)
    }

    fn on_channel_switched(&mut self, info: fidl_mlme::ChannelSwitchInfo) {
        self.bss.chan.primary = info.new_channel;
        self.last_channel_switch_time.replace(now());
    }

    fn on_wmm_status_resp(
        &mut self,
        status: zx::zx_status_t,
        resp: fidl_internal::WmmStatusResponse,
    ) {
        if status == zx::sys::ZX_OK {
            let wmm_param = self.wmm_param.get_or_insert_with(|| ie::WmmParam::default());
            let mut wmm_info = wmm_param.wmm_info.ap_wmm_info();
            wmm_info.set_uapsd(resp.apsd);
            wmm_param.wmm_info.0 = wmm_info.0;
            update_wmm_ac_param(&mut wmm_param.ac_be_params, &resp.ac_be_params);
            update_wmm_ac_param(&mut wmm_param.ac_bk_params, &resp.ac_bk_params);
            update_wmm_ac_param(&mut wmm_param.ac_vo_params, &resp.ac_vo_params);
            update_wmm_ac_param(&mut wmm_param.ac_vi_params, &resp.ac_vi_params);
        }
    }

    fn handle_timeout(
        self,
        event_id: EventId,
        event: Event,
        state_change_ctx: &mut Option<StateChangeContext>,
        context: &mut Context,
    ) -> Result<Self, Idle> {
        match self.link_state.handle_timeout(event_id, event, state_change_ctx, context) {
            Ok(link_state) => Ok(Associated { link_state, ..self }),
            Err(failure_reason) => {
                report_connect_finished(
                    self.responder,
                    context,
                    EstablishRsnaFailure { auth_method: self.auth_method, reason: failure_reason }
                        .into(),
                );
                send_deauthenticate_request(&self.bss, &context.mlme_sink);
                Err(Idle { cfg: self.cfg })
            }
        }
    }
}

impl ClientState {
    pub fn new(cfg: ClientConfig) -> Self {
        Self::from(State::new(Idle { cfg }))
    }

    fn state_name(&self) -> &'static str {
        match self {
            Self::Idle(_) => IDLE_STATE,
            Self::Joining(_) => JOINING_STATE,
            Self::Authenticating(_) => AUTHENTICATING_STATE,
            Self::Associating(_) => ASSOCIATING_STATE,
            Self::Associated(state) => match state.link_state {
                LinkState::EstablishingRsna(_) => RSNA_STATE,
                LinkState::LinkUp(_) => LINK_UP_STATE,
                _ => unreachable!(),
            },
        }
    }

    pub fn on_mlme_event(self, event: MlmeEvent, context: &mut Context) -> Self {
        let start_state = self.state_name();
        let mut state_change_ctx: Option<StateChangeContext> = None;

        let new_state = match self {
            Self::Idle(_) => {
                match event {
                    MlmeEvent::OnWmmStatusResp { .. } => (),
                    _ => warn!("Unexpected MLME message while Idle: {:?}", event),
                }
                self
            }
            Self::Joining(state) => match event {
                MlmeEvent::JoinConf { resp } => {
                    let (transition, joining) = state.release_data();
                    match joining.on_join_conf(resp, &mut state_change_ctx, context) {
                        Ok(authenticating) => transition.to(authenticating).into(),
                        Err(idle) => transition.to(idle).into(),
                    }
                }
                _ => state.into(),
            },
            Self::Authenticating(state) => match event {
                MlmeEvent::AuthenticateConf { resp } => {
                    let (transition, authenticating) = state.release_data();
                    match authenticating.on_authenticate_conf(resp, &mut state_change_ctx, context)
                    {
                        Ok(associating) => transition.to(associating).into(),
                        Err(idle) => transition.to(idle).into(),
                    }
                }
                MlmeEvent::OnPmkAvailable { info } => {
                    let (transition, mut authenticating) = state.release_data();
                    if let Err(e) = authenticating.on_pmk_available(info) {
                        error!("Failed to process OnPmkAvailable: {:?}", e);
                    }
                    transition.to(authenticating).into()
                }
                MlmeEvent::OnSaeHandshakeInd { ind } => {
                    let (transition, mut authenticating) = state.release_data();
                    if let Err(e) = authenticating.on_sae_handshake_ind(ind, context) {
                        error!("Failed to process SaeHandshakeInd: {:?}", e);
                    }
                    transition.to(authenticating).into()
                }
                MlmeEvent::OnSaeFrameRx { frame } => {
                    let (transition, mut authenticating) = state.release_data();
                    if let Err(e) = authenticating.on_sae_frame_rx(frame, context) {
                        error!("Failed to process SaeFrameRx: {:?}", e);
                    }
                    transition.to(authenticating).into()
                }
                MlmeEvent::DeauthenticateInd { ind } => {
                    let (transition, authenticating) = state.release_data();
                    let idle =
                        authenticating.on_deauthenticate_ind(ind, &mut state_change_ctx, context);
                    transition.to(idle).into()
                }
                _ => state.into(),
            },
            Self::Associating(state) => match event {
                MlmeEvent::AssociateConf { resp } => {
                    let (transition, associating) = state.release_data();
                    match associating.on_associate_conf(resp, &mut state_change_ctx, context) {
                        Ok(associated) => transition.to(associated).into(),
                        Err(idle) => transition.to(idle).into(),
                    }
                }
                MlmeEvent::DeauthenticateInd { ind } => {
                    let (transition, associating) = state.release_data();
                    let idle =
                        associating.on_deauthenticate_ind(ind, &mut state_change_ctx, context);
                    transition.to(idle).into()
                }
                MlmeEvent::DisassociateInd { ind } => {
                    let (transition, associating) = state.release_data();
                    let idle = associating.on_disassociate_ind(ind, &mut state_change_ctx, context);
                    transition.to(idle).into()
                }
                MlmeEvent::OnSaeHandshakeInd { ind } => {
                    let (transition, mut associating) = state.release_data();
                    if let Err(e) = associating.on_sae_handshake_ind(ind, context) {
                        error!("Failed to process SaeHandshakeInd: {:?}", e);
                    }
                    transition.to(associating).into()
                }
                MlmeEvent::OnSaeFrameRx { frame } => {
                    let (transition, mut associating) = state.release_data();
                    if let Err(e) = associating.on_sae_frame_rx(frame, context) {
                        error!("Failed to process SaeFrameRx: {:?}", e);
                    }
                    transition.to(associating).into()
                }
                _ => state.into(),
            },
            Self::Associated(mut state) => match event {
                MlmeEvent::DisassociateInd { ind } => {
                    let (transition, associated) = state.release_data();
                    let associating =
                        associated.on_disassociate_ind(ind, &mut state_change_ctx, context);
                    transition.to(associating).into()
                }
                MlmeEvent::DeauthenticateInd { ind } => {
                    let (transition, associated) = state.release_data();
                    let idle =
                        associated.on_deauthenticate_ind(ind, &mut state_change_ctx, context);
                    transition.to(idle).into()
                }
                MlmeEvent::SignalReport { ind } => {
                    state.last_rssi = ind.rssi_dbm;
                    state.last_snr = ind.snr_db;
                    state.last_signal_report_time = now();
                    state.into()
                }
                MlmeEvent::EapolInd { ind } => {
                    let (transition, associated) = state.release_data();
                    match associated.on_eapol_ind(ind, &mut state_change_ctx, context) {
                        Ok(associated) => transition.to(associated).into(),
                        Err(idle) => transition.to(idle).into(),
                    }
                }
                MlmeEvent::EapolConf { resp } => {
                    let (transition, associated) = state.release_data();
                    match associated.on_eapol_conf(resp, &mut state_change_ctx, context) {
                        Ok(associated) => transition.to(associated).into(),
                        Err(idle) => transition.to(idle).into(),
                    }
                }
                MlmeEvent::OnChannelSwitched { info } => {
                    state.on_channel_switched(info);
                    state.into()
                }
                MlmeEvent::OnWmmStatusResp { status, resp } => {
                    state.on_wmm_status_resp(status, resp);
                    state.into()
                }
                _ => state.into(),
            },
        };

        log_state_change(start_state, &new_state, state_change_ctx, context);
        new_state
    }

    pub fn handle_timeout(self, event_id: EventId, event: Event, context: &mut Context) -> Self {
        let start_state = self.state_name();
        let mut state_change_ctx: Option<StateChangeContext> = None;

        let new_state = match self {
            Self::Authenticating(state) => {
                let (transition, authenticating) = state.release_data();
                match authenticating.handle_timeout(event_id, event, &mut state_change_ctx, context)
                {
                    Ok(authenticating) => transition.to(authenticating).into(),
                    Err(idle) => transition.to(idle).into(),
                }
            }
            Self::Associating(state) => {
                let (transition, associating) = state.release_data();
                match associating.handle_timeout(event_id, event, &mut state_change_ctx, context) {
                    Ok(associating) => transition.to(associating).into(),
                    Err(idle) => transition.to(idle).into(),
                }
            }
            Self::Associated(state) => {
                let (transition, associated) = state.release_data();
                match associated.handle_timeout(event_id, event, &mut state_change_ctx, context) {
                    Ok(associated) => transition.to(associated).into(),
                    Err(idle) => transition.to(idle).into(),
                }
            }
            _ => self,
        };

        log_state_change(start_state, &new_state, state_change_ctx, context);
        new_state
    }

    pub fn connect(self, cmd: ConnectCommand, context: &mut Context) -> Self {
        let (chan, cap) = match derive_join_channel_and_capabilities(
            Channel::from_fidl(cmd.bss.chan),
            cmd.radio_cfg.cbw,
            cmd.bss.rates(),
            &context.device_info,
        ) {
            Ok(chan_and_cap) => chan_and_cap,
            Err(e) => {
                error!("Failed building join capabilities: {}", e);
                return self;
            }
        };

        let cap = if context.is_softmac { Some(cap) } else { None };

        // Derive RSN (for WPA2) or Vendor IEs (for WPA1) or neither(WEP/non-protected).
        let protection_ie = match build_protection_ie(&cmd.protection) {
            Ok(ie) => ie,
            Err(e) => {
                error!("Failed to build protection IEs: {}", e);
                return self;
            }
        };

        let start_state = self.state_name();
        let cfg = self.disconnect_internal(context);

        let mut selected_bss = cmd.bss.clone();
        let (phy_to_use, cbw_to_use) =
            derive_phy_cbw(&selected_bss, &context.device_info, &cmd.radio_cfg);
        selected_bss.chan.cbw = cbw_to_use;

        context.mlme_sink.send(MlmeRequest::Join(fidl_mlme::JoinRequest {
            selected_bss: selected_bss.to_fidl(),
            join_failure_timeout: DEFAULT_JOIN_FAILURE_TIMEOUT,
            nav_sync_delay: 0,
            op_rates: vec![],
            phy: phy_to_use,
            cbw: cbw_to_use,
        }));
        context.att_id += 1;

        let msg = connect_cmd_inspect_summary(&cmd);
        inspect_log!(context.inspect.state_events.lock(), {
            from: start_state,
            to: JOINING_STATE,
            ctx: msg,
            bssid: cmd.bss.bssid.to_mac_str(),
            bssid_hash: context.inspect.hasher.hash_mac_addr(&cmd.bss.bssid),
            ssid: cmd.bss.ssid().to_ssid_str(),
            ssid_hash: context.inspect.hasher.hash(cmd.bss.ssid()),
        });
        let state = Self::new(cfg.clone());
        match state {
            Self::Idle(state) => {
                state.transition_to(Joining { cfg, cmd, chan, cap, protection_ie }).into()
            }
            _ => unreachable!(),
        }
    }

    pub fn disconnect(
        self,
        context: &mut Context,
        user_disconnect_reason: fidl_sme::UserDisconnectReason,
    ) -> Self {
        let mut disconnected_from_link_up = false;
        let disconnect_source = DisconnectSource::User(user_disconnect_reason);
        if let Self::Associated(state) = &self {
            if let LinkState::LinkUp(link_up) = &state.link_state {
                disconnected_from_link_up = true;
                let disconnect_info = DisconnectInfo {
                    connected_duration: link_up.connected_duration(),
                    last_rssi: state.last_rssi,
                    last_snr: state.last_snr,
                    bssid: state.bss.bssid,
                    ssid: state.bss.ssid().to_vec(),
                    protection: state.bss.protection(),
                    wsc: state.bss.probe_resp_wsc(),
                    channel: Channel::from_fidl(state.bss.chan),
                    disconnect_source,
                    time_since_channel_switch: state.last_channel_switch_time.map(|t| now() - t),
                };
                context.info.report_disconnect(disconnect_info);
            }
        }
        let start_state = self.state_name();
        let new_state = Self::new(self.disconnect_internal(context));

        let msg =
            format!("received disconnect command from user; reason {:?}", user_disconnect_reason);
        let state_change_ctx = Some(if disconnected_from_link_up {
            StateChangeContext::Disconnect { msg, disconnect_source }
        } else {
            StateChangeContext::Msg(msg)
        });
        log_state_change(start_state, &new_state, state_change_ctx, context);
        new_state
    }

    fn disconnect_internal(self, context: &mut Context) -> ClientConfig {
        match self {
            Self::Idle(state) => state.cfg,
            Self::Joining(state) => {
                let (_, state) = state.release_data();
                report_connect_finished(state.cmd.responder, context, ConnectResult::Canceled);
                state.cfg
            }
            Self::Authenticating(state) => {
                let (_, state) = state.release_data();
                report_connect_finished(state.cmd.responder, context, ConnectResult::Canceled);
                state.cfg
            }
            Self::Associating(state) => {
                let (_, state) = state.release_data();
                report_connect_finished(state.cmd.responder, context, ConnectResult::Canceled);
                send_deauthenticate_request(&state.cmd.bss, &context.mlme_sink);
                state.cfg
            }
            Self::Associated(state) => {
                send_deauthenticate_request(&state.bss, &context.mlme_sink);
                state.cfg
            }
        }
    }

    // Cancel any connect that is in progress. No-op if client is already idle or connected.
    pub fn cancel_ongoing_connect(self, context: &mut Context) -> Self {
        // Only move to idle if client is not already connected. Technically, SME being in
        // transition state does not necessarily mean that a (manual) connect attempt is
        // in progress (since DisassociateInd moves SME to transition state). However, the
        // main thing we are concerned about is that we don't disconnect from an already
        // connected state until the new connect attempt succeeds in selecting BSS.
        if self.in_transition_state() {
            Self::new(self.disconnect_internal(context))
        } else {
            self
        }
    }

    fn in_transition_state(&self) -> bool {
        match self {
            Self::Idle(_) => false,
            Self::Associated(state) => match state.link_state {
                LinkState::LinkUp { .. } => false,
                _ => true,
            },
            _ => true,
        }
    }

    pub fn status(&self) -> Status {
        match self {
            Self::Idle(_) => Status { connected_to: None, connecting_to: None },
            Self::Joining(joining) => {
                Status { connected_to: None, connecting_to: Some(joining.cmd.bss.ssid().to_vec()) }
            }
            Self::Authenticating(authenticating) => Status {
                connected_to: None,
                connecting_to: Some(authenticating.cmd.bss.ssid().to_vec()),
            },
            Self::Associating(associating) => Status {
                connected_to: None,
                connecting_to: Some(associating.cmd.bss.ssid().to_vec()),
            },
            Self::Associated(associated) => match associated.link_state {
                LinkState::EstablishingRsna { .. } => Status {
                    connected_to: None,
                    connecting_to: Some(associated.bss.ssid().to_vec()),
                },
                LinkState::LinkUp { .. } => Status {
                    connected_to: {
                        let mut bss = associated
                            .cfg
                            .convert_bss_description(&associated.bss, associated.wmm_param);
                        bss.rssi_dbm = associated.last_rssi;
                        bss.snr_db = associated.last_snr;
                        bss.signal_report_time = associated.last_signal_report_time;
                        Some(bss)
                    },
                    connecting_to: None,
                },
                _ => unreachable!(),
            },
        }
    }
}

fn update_wmm_ac_param(ac_params: &mut ie::WmmAcParams, update: &fidl_internal::WmmAcParams) {
    ac_params.aci_aifsn.set_aifsn(update.aifsn);
    ac_params.aci_aifsn.set_acm(update.acm);
    ac_params.ecw_min_max.set_ecw_min(update.ecw_min);
    ac_params.ecw_min_max.set_ecw_max(update.ecw_max);
    ac_params.txop_limit = update.txop_limit;
}

fn process_sae_updates(updates: UpdateSink, peer_sta_address: [u8; 6], context: &mut Context) {
    for update in updates {
        match update {
            SecAssocUpdate::TxSaeFrame(frame) => {
                context.mlme_sink.send(MlmeRequest::SaeFrameTx(frame));
            }
            SecAssocUpdate::SaeAuthStatus(status) => context.mlme_sink.send(
                MlmeRequest::SaeHandshakeResp(fidl_mlme::SaeHandshakeResponse {
                    peer_sta_address,
                    status_code: match status {
                        AuthStatus::Success => fidl_ieee80211::StatusCode::Success,
                        AuthStatus::Rejected => {
                            fidl_ieee80211::StatusCode::RefusedReasonUnspecified
                        }
                        AuthStatus::InternalError => {
                            fidl_ieee80211::StatusCode::RefusedReasonUnspecified
                        }
                    },
                }),
            ),
            SecAssocUpdate::ScheduleSaeTimeout(id) => {
                context.timer.schedule(event::SaeTimeout(id));
            }
            _ => (),
        }
    }
}

fn process_sae_handshake_ind(
    protection: &mut Protection,
    ind: fidl_mlme::SaeHandshakeIndication,
    context: &mut Context,
) -> Result<(), anyhow::Error> {
    let supplicant = match protection {
        Protection::Rsna(rsna) => &mut rsna.supplicant,
        _ => bail!("Unexpected SAE handshake indication"),
    };

    let mut updates = UpdateSink::default();
    supplicant.on_sae_handshake_ind(&mut updates)?;
    process_sae_updates(updates, ind.peer_sta_address, context);
    Ok(())
}

fn process_sae_frame_rx(
    protection: &mut Protection,
    frame: fidl_mlme::SaeFrame,
    context: &mut Context,
) -> Result<(), anyhow::Error> {
    let peer_sta_address = frame.peer_sta_address.clone();
    let supplicant = match protection {
        Protection::Rsna(rsna) => &mut rsna.supplicant,
        _ => bail!("Unexpected SAE frame recieved"),
    };

    let mut updates = UpdateSink::default();
    supplicant.on_sae_frame_rx(&mut updates, frame)?;
    process_sae_updates(updates, peer_sta_address, context);
    Ok(())
}

fn process_sae_timeout(
    protection: &mut Protection,
    bssid: [u8; 6],
    event: Event,
    context: &mut Context,
) -> Result<(), anyhow::Error> {
    match event {
        Event::SaeTimeout(timer) => {
            let supplicant = match protection {
                Protection::Rsna(rsna) => &mut rsna.supplicant,
                // Ignore timeouts if we're not using SAE.
                _ => return Ok(()),
            };

            let mut updates = UpdateSink::default();
            supplicant.on_sae_timeout(&mut updates, timer.0)?;
            process_sae_updates(updates, bssid, context);
        }
        _ => (),
    }
    Ok(())
}

fn log_state_change(
    start_state: &str,
    new_state: &ClientState,
    state_change_ctx: Option<StateChangeContext>,
    context: &mut Context,
) {
    if start_state == new_state.state_name() && state_change_ctx.is_none() {
        return;
    }

    match state_change_ctx {
        Some(inner) => match inner {
            // Only log the `disconnect_ctx` if an operation had an effect of moving from
            // non-idle state to idle state. This is so that the client that consumes
            // `disconnect_ctx` does not log a disconnect event when it's effectively no-op.
            StateChangeContext::Disconnect { msg, disconnect_source }
                if start_state != IDLE_STATE =>
            {
                info!(
                    "{} => {}, ctx: `{}`, disconnect_source: {:?}",
                    start_state,
                    new_state.state_name(),
                    msg,
                    disconnect_source,
                );

                inspect_log!(context.inspect.state_events.lock(), {
                    from: start_state,
                    to: new_state.state_name(),
                    ctx: msg,
                    disconnect_ctx: {
                        reason_code: disconnect_source.reason_code() as u64,
                        locally_initiated: disconnect_source.locally_initiated(),
                    }
                });
            }
            StateChangeContext::Disconnect { msg, .. } | StateChangeContext::Msg(msg) => {
                inspect_log!(context.inspect.state_events.lock(), {
                    from: start_state,
                    to: new_state.state_name(),
                    ctx: msg,
                });
            }
        },
        None => {
            inspect_log!(context.inspect.state_events.lock(), {
                from: start_state,
                to: new_state.state_name(),
            });
        }
    }
}

fn install_wep_key(context: &mut Context, bssid: [u8; 6], key: &wep_deprecated::Key) {
    let cipher_suite = match key {
        wep_deprecated::Key::Bits40(_) => cipher::WEP_40,
        wep_deprecated::Key::Bits104(_) => cipher::WEP_104,
    };
    // unwrap() is safe, OUI is defined in RSN and always compatible with ciphers.
    let cipher = cipher::Cipher::new_dot11(cipher_suite);
    inspect_log!(context.inspect.rsn_events.lock(), {
        derived_key: "WEP",
        cipher: format!("{:?}", cipher),
        key_index: 0,
    });
    context
        .mlme_sink
        .send(MlmeRequest::SetKeys(wep_deprecated::make_mlme_set_keys_request(bssid, key)));
}

/// Custom logging for ConnectCommand because its normal full debug string is too large, and we
/// want to reduce how much we log in memory for Inspect. Additionally, in the future, we'd need
/// to anonymize information like BSSID and SSID.
fn connect_cmd_inspect_summary(cmd: &ConnectCommand) -> String {
    let bss = &cmd.bss;
    format!(
        "ConnectCmd {{ \
         cap: {cap:?}, rates: {rates:?}, \
         protected: {protected:?}, chan: {chan:?}, \
         rssi: {rssi:?}, ht_cap: {ht_cap:?}, ht_op: {ht_op:?}, \
         vht_cap: {vht_cap:?}, vht_op: {vht_op:?} }}",
        cap = bss.cap,
        rates = bss.rates(),
        protected = bss.rsne().is_some(),
        chan = bss.chan,
        rssi = bss.rssi_dbm,
        ht_cap = bss.ht_cap().is_some(),
        ht_op = bss.ht_op().is_some(),
        vht_cap = bss.vht_cap().is_some(),
        vht_op = bss.vht_op().is_some()
    )
}

fn send_deauthenticate_request(current_bss: &BssDescription, mlme_sink: &MlmeSink) {
    mlme_sink.send(MlmeRequest::Deauthenticate(fidl_mlme::DeauthenticateRequest {
        peer_sta_address: current_bss.bssid.clone(),
        reason_code: fidl_ieee80211::ReasonCode::StaLeaving,
    }));
}

fn send_mlme_assoc_req(
    bssid: Bssid,
    capabilities: Option<&ClientCapabilities>,
    protection_ie: &Option<ProtectionIe>,
    mlme_sink: &MlmeSink,
) {
    assert_eq_size!(ie::HtCapabilities, [u8; fidl_internal::HT_CAP_LEN as usize]);
    let ht_cap = capabilities.map_or(None, |c| {
        c.0.ht_cap
            .map(|h| fidl_internal::HtCapabilities { bytes: h.as_bytes().try_into().unwrap() })
    });

    assert_eq_size!(ie::VhtCapabilities, [u8; fidl_internal::VHT_CAP_LEN as usize]);
    let vht_cap = capabilities.map_or(None, |c| {
        c.0.vht_cap
            .map(|v| fidl_internal::VhtCapabilities { bytes: v.as_bytes().try_into().unwrap() })
    });
    let (rsne, vendor_ies) = match protection_ie.as_ref() {
        Some(ProtectionIe::Rsne(vec)) => (Some(vec.to_vec()), None),
        Some(ProtectionIe::VendorIes(vec)) => (None, Some(vec.to_vec())),
        None => (None, None),
    };
    let req = fidl_mlme::AssociateRequest {
        peer_sta_address: bssid.0,
        cap_info: capabilities.map_or(0, |c| c.0.cap_info.raw()),
        rates: capabilities.map_or_else(|| vec![], |c| c.0.rates.as_bytes().to_vec()),
        // TODO(fxbug.dev/43938): populate `qos_capable` field from device info
        qos_capable: ht_cap.is_some(),
        qos_info: 0,
        ht_cap: ht_cap.map(Box::new),
        vht_cap: vht_cap.map(Box::new),
        rsne,
        vendor_ies,
    };
    mlme_sink.send(MlmeRequest::Associate(req))
}

fn now() -> zx::Time {
    zx::Time::get_monotonic()
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::format_err;
    use fidl_fuchsia_wlan_common as fidl_common;
    use fuchsia_inspect::{assert_inspect_tree, testing::AnyProperty, Inspector};
    use futures::channel::{mpsc, oneshot};
    use link_state::{EstablishingRsna, LinkUp};
    use std::sync::Arc;
    use wlan_common::{
        assert_variant,
        bss::Protection as BssProtection,
        fake_bss,
        hasher::WlanHasher,
        ie::{
            fake_ies::{
                fake_probe_resp_wsc_ie, fake_probe_resp_wsc_ie_bytes,
                get_vendor_ie_bytes_for_wsc_ie,
            },
            rsn::rsne::Rsne,
        },
        test_utils::fake_stas::IesOverrides,
        RadioConfig,
    };
    use wlan_rsn::{key::exchange::Key, rsna::SecAssocStatus};
    use wlan_rsn::{
        rsna::{SecAssocUpdate, UpdateSink},
        NegotiatedProtection,
    };

    use crate::client::test_utils::{
        create_assoc_conf, create_auth_conf, create_join_conf, create_on_wmm_status_resp,
        expect_stream_empty, fake_negotiated_channel_and_capabilities, fake_wmm_param,
        mock_psk_supplicant, MockSupplicant, MockSupplicantController,
    };
    use crate::client::{info::InfoReporter, inspect, rsn::Rsna, InfoEvent, InfoSink, TimeStream};
    use crate::test_utils::make_wpa1_ie;

    use crate::{test_utils, timer, InfoStream, MlmeStream};

    #[test]
    fn associate_happy_path_unprotected() {
        let mut h = TestHelper::new();

        let state = idle_state();
        let (command, receiver) = connect_command_one();
        let bssid = command.bss.bssid.clone();

        // Issue a "connect" command
        let state = state.connect(command, &mut h.context);

        expect_join_request(&mut h.mlme_stream, bssid);

        // (mlme->sme) Send a JoinConf as a response
        let join_conf = create_join_conf(fidl_mlme::JoinResultCode::Success);
        let state = state.on_mlme_event(join_conf, &mut h.context);

        expect_auth_req(&mut h.mlme_stream, bssid);

        // (mlme->sme) Send an AuthenticateConf as a response
        let auth_conf = create_auth_conf(bssid.clone(), fidl_mlme::AuthenticateResultCode::Success);
        let state = state.on_mlme_event(auth_conf, &mut h.context);

        expect_assoc_req(&mut h.mlme_stream, bssid);

        // (mlme->sme) Send an AssociateConf
        let assoc_conf = create_assoc_conf(fidl_mlme::AssociateResultCode::Success);
        let _state = state.on_mlme_event(assoc_conf, &mut h.context);

        // User should be notified that we are connected
        expect_result(receiver, ConnectResult::Success);

        assert_variant!(h.info_stream.try_next(), Ok(Some(InfoEvent::ConnectionPing(..))));
    }

    #[test]
    fn connect_to_wep_network() {
        let mut h = TestHelper::new();

        let state = idle_state();
        let (command, receiver) = connect_command_wep();
        let bssid = command.bss.bssid.clone();

        // Issue a "connect" command
        let state = state.connect(command, &mut h.context);

        expect_join_request(&mut h.mlme_stream, bssid);

        // (mlme->sme) Send a JoinConf as a response
        let join_conf = create_join_conf(fidl_mlme::JoinResultCode::Success);
        let state = state.on_mlme_event(join_conf, &mut h.context);

        // (sme->mlme) Expect an SetKeysRequest
        expect_set_wep_key(&mut h.mlme_stream, bssid, vec![3; 5]);
        // (sme->mlme) Expect an AuthenticateRequest
        assert_variant!(&mut h.mlme_stream.try_next(),
            Ok(Some(MlmeRequest::Authenticate(req))) => {
                assert_eq!(fidl_mlme::AuthenticationTypes::SharedKey, req.auth_type);
                assert_eq!(bssid, req.peer_sta_address);
            }
        );

        // (mlme->sme) Send an AuthenticateConf as a response
        let auth_conf = create_auth_conf(bssid.clone(), fidl_mlme::AuthenticateResultCode::Success);
        let state = state.on_mlme_event(auth_conf, &mut h.context);

        expect_assoc_req(&mut h.mlme_stream, bssid);

        // (mlme->sme) Send an AssociateConf
        let assoc_conf = create_assoc_conf(fidl_mlme::AssociateResultCode::Success);
        let _state = state.on_mlme_event(assoc_conf, &mut h.context);

        // User should be notified that we are connected
        expect_result(receiver, ConnectResult::Success);

        assert_variant!(h.info_stream.try_next(), Ok(Some(InfoEvent::ConnectionPing(..))));
    }

    #[test]
    fn connect_to_wpa1_network() {
        let mut h = TestHelper::new();
        let (supplicant, suppl_mock) = mock_psk_supplicant();

        let state = idle_state();
        let (command, receiver) = connect_command_wpa1(supplicant);
        let bssid = command.bss.bssid.clone();

        // Issue a "connect" command
        let state = state.connect(command, &mut h.context);

        expect_join_request(&mut h.mlme_stream, bssid);

        // (mlme->sme) Send a JoinConf as a response
        let join_conf = create_join_conf(fidl_mlme::JoinResultCode::Success);
        let state = state.on_mlme_event(join_conf, &mut h.context);

        expect_auth_req(&mut h.mlme_stream, bssid);

        // (mlme->sme) Send an AuthenticateConf as a response
        let auth_conf = create_auth_conf(bssid.clone(), fidl_mlme::AuthenticateResultCode::Success);
        let state = state.on_mlme_event(auth_conf, &mut h.context);

        expect_assoc_req(&mut h.mlme_stream, bssid);

        // (mlme->sme) Send an AssociateConf
        let assoc_conf = create_assoc_conf(fidl_mlme::AssociateResultCode::Success);
        let state = state.on_mlme_event(assoc_conf, &mut h.context);
        expect_finalize_association_req(
            &mut h.mlme_stream,
            fake_negotiated_channel_and_capabilities(),
        );

        assert!(suppl_mock.is_supplicant_started());

        // (mlme->sme) Send an EapolInd, mock supplicant with key frame
        let update = SecAssocUpdate::TxEapolKeyFrame(test_utils::eapol_key_frame());
        let state = on_eapol_ind(state, &mut h, bssid, &suppl_mock, vec![update]);

        expect_eapol_req(&mut h.mlme_stream, bssid);

        // (mlme->sme) Send an EapolInd, mock supplicant with keys
        let ptk = SecAssocUpdate::Key(Key::Ptk(test_utils::wpa1_ptk()));
        let gtk = SecAssocUpdate::Key(Key::Gtk(test_utils::wpa1_gtk()));
        let state = on_eapol_ind(state, &mut h, bssid, &suppl_mock, vec![ptk, gtk]);

        expect_set_wpa1_ptk(&mut h.mlme_stream, bssid);
        expect_set_wpa1_gtk(&mut h.mlme_stream);

        // (mlme->sme) Send an EapolInd, mock supplicant with completion status
        let update = SecAssocUpdate::Status(SecAssocStatus::EssSaEstablished);
        let _state = on_eapol_ind(state, &mut h, bssid, &suppl_mock, vec![update]);

        expect_set_ctrl_port(&mut h.mlme_stream, bssid, fidl_mlme::ControlledPortState::Open);
        expect_result(receiver, ConnectResult::Success);
        assert_variant!(h.info_stream.try_next(), Ok(Some(InfoEvent::ConnectionPing(..))));
    }

    #[test]
    fn associate_happy_path_protected() {
        let mut h = TestHelper::new();
        let (supplicant, suppl_mock) = mock_psk_supplicant();

        let state = idle_state();
        let (command, receiver) = connect_command_wpa2(supplicant);
        let bssid = command.bss.bssid.clone();

        // Issue a "connect" command
        let state = state.connect(command, &mut h.context);

        expect_join_request(&mut h.mlme_stream, bssid);

        // (mlme->sme) Send a JoinConf as a response
        let join_conf = create_join_conf(fidl_mlme::JoinResultCode::Success);
        let state = state.on_mlme_event(join_conf, &mut h.context);

        expect_auth_req(&mut h.mlme_stream, bssid);

        // (mlme->sme) Send an AuthenticateConf as a response
        let auth_conf = create_auth_conf(bssid.clone(), fidl_mlme::AuthenticateResultCode::Success);
        let state = state.on_mlme_event(auth_conf, &mut h.context);

        expect_assoc_req(&mut h.mlme_stream, bssid);

        // (mlme->sme) Send an AssociateConf
        let assoc_conf = create_assoc_conf(fidl_mlme::AssociateResultCode::Success);
        let state = state.on_mlme_event(assoc_conf, &mut h.context);
        expect_finalize_association_req(
            &mut h.mlme_stream,
            fake_negotiated_channel_and_capabilities(),
        );

        assert!(suppl_mock.is_supplicant_started());

        // (mlme->sme) Send an EapolInd, mock supplicant with key frame
        let update = SecAssocUpdate::TxEapolKeyFrame(test_utils::eapol_key_frame());
        let state = on_eapol_ind(state, &mut h, bssid, &suppl_mock, vec![update]);

        expect_eapol_req(&mut h.mlme_stream, bssid);

        // (mlme->sme) Send an EapolInd, mock supplicant with keys
        let ptk = SecAssocUpdate::Key(Key::Ptk(test_utils::ptk()));
        let gtk = SecAssocUpdate::Key(Key::Gtk(test_utils::gtk()));
        let state = on_eapol_ind(state, &mut h, bssid, &suppl_mock, vec![ptk, gtk]);

        expect_set_ptk(&mut h.mlme_stream, bssid);
        expect_set_gtk(&mut h.mlme_stream);

        // (mlme->sme) Send an EapolInd, mock supplicant with completion status
        let update = SecAssocUpdate::Status(SecAssocStatus::EssSaEstablished);
        let _state = on_eapol_ind(state, &mut h, bssid, &suppl_mock, vec![update]);

        expect_set_ctrl_port(&mut h.mlme_stream, bssid, fidl_mlme::ControlledPortState::Open);
        expect_result(receiver, ConnectResult::Success);
        assert_variant!(h.info_stream.try_next(), Ok(Some(InfoEvent::ConnectionPing(..))));
    }

    #[test]
    fn join_failure() {
        let mut h = TestHelper::new();

        let (cmd, receiver) = connect_command_one();
        // Start in a "Joining" state
        let state = ClientState::from(testing::new_state(Joining {
            cfg: ClientConfig::default(),
            cmd,
            chan: fake_channel(),
            cap: None,
            protection_ie: None,
        }));

        // (mlme->sme) Send an unsuccessful JoinConf
        let join_conf = MlmeEvent::JoinConf {
            resp: fidl_mlme::JoinConfirm {
                result_code: fidl_mlme::JoinResultCode::JoinFailureTimeout,
            },
        };
        let state = state.on_mlme_event(join_conf, &mut h.context);
        assert_idle(state);

        let result = ConnectResult::Failed(ConnectFailure::JoinFailure(
            fidl_mlme::JoinResultCode::JoinFailureTimeout,
        ));
        // User should be notified that connection attempt failed
        expect_result(receiver, result.clone());
    }

    #[test]
    fn authenticate_failure() {
        let mut h = TestHelper::new();

        let (cmd, receiver) = connect_command_one();

        // Start in an "Authenticating" state
        let state = ClientState::from(testing::new_state(Authenticating {
            cfg: ClientConfig::default(),
            cmd,
            chan: fake_channel(),
            cap: None,
            protection_ie: None,
        }));

        // (mlme->sme) Send an unsuccessful AuthenticateConf
        let auth_conf = MlmeEvent::AuthenticateConf {
            resp: fidl_mlme::AuthenticateConfirm {
                peer_sta_address: connect_command_one().0.bss.bssid,
                auth_type: fidl_mlme::AuthenticationTypes::OpenSystem,
                result_code: fidl_mlme::AuthenticateResultCode::Refused,
            },
        };
        let state = state.on_mlme_event(auth_conf, &mut h.context);
        assert_idle(state);

        let result = ConnectResult::Failed(ConnectFailure::AuthenticationFailure(
            fidl_mlme::AuthenticateResultCode::Refused,
        ));
        // User should be notified that connection attempt failed
        expect_result(receiver, result.clone());
    }

    #[test]
    fn associate_failure() {
        let mut h = TestHelper::new();

        let (cmd, receiver) = connect_command_one();
        let bss_protection = cmd.bss.protection();

        // Start in an "Associating" state
        let state = ClientState::from(testing::new_state(Associating {
            cfg: ClientConfig::default(),
            cmd,
            chan: fake_channel(),
            cap: None,
            protection_ie: None,
        }));

        // (mlme->sme) Send an unsuccessful AssociateConf
        let assoc_conf =
            create_assoc_conf(fidl_mlme::AssociateResultCode::RefusedReasonUnspecified);
        let state = state.on_mlme_event(assoc_conf, &mut h.context);
        assert_idle(state);

        let result = ConnectResult::Failed(
            AssociationFailure {
                bss_protection,
                code: fidl_mlme::AssociateResultCode::RefusedReasonUnspecified,
            }
            .into(),
        );
        // User should be notified that connection attempt failed
        expect_result(receiver, result.clone());
    }

    #[test]
    fn connect_while_joining() {
        let mut h = TestHelper::new();
        let (cmd_one, receiver_one) = connect_command_one();
        let state = joining_state(cmd_one);
        let (cmd_two, _receiver_two) = connect_command_two();
        let state = state.connect(cmd_two, &mut h.context);
        expect_result(receiver_one, ConnectResult::Canceled);
        expect_join_request(&mut h.mlme_stream, connect_command_two().0.bss.bssid);
        assert_joining(state, &connect_command_two().0.bss);
    }

    #[test]
    fn connect_while_authenticating() {
        let mut h = TestHelper::new();
        let (cmd_one, receiver_one) = connect_command_one();
        let state = authenticating_state(cmd_one);
        let (cmd_two, _receiver_two) = connect_command_two();
        let state = state.connect(cmd_two, &mut h.context);
        expect_result(receiver_one, ConnectResult::Canceled);
        expect_join_request(&mut h.mlme_stream, connect_command_two().0.bss.bssid);
        assert_joining(state, &connect_command_two().0.bss);
    }

    #[test]
    fn connect_while_associating() {
        let mut h = TestHelper::new();
        let (cmd_one, receiver_one) = connect_command_one();
        let state = associating_state(cmd_one);
        let (cmd_two, _receiver_two) = connect_command_two();
        let state = state.connect(cmd_two, &mut h.context);
        let state = exchange_deauth(state, &mut h);
        expect_result(receiver_one, ConnectResult::Canceled);
        expect_join_request(&mut h.mlme_stream, connect_command_two().0.bss.bssid);
        assert_joining(state, &connect_command_two().0.bss);
    }

    #[test]
    fn deauth_while_authing() {
        let mut h = TestHelper::new();
        let (cmd_one, receiver_one) = connect_command_one();
        let state = authenticating_state(cmd_one);
        let deauth_ind = MlmeEvent::DeauthenticateInd {
            ind: fidl_mlme::DeauthenticateIndication {
                peer_sta_address: [7, 7, 7, 7, 7, 7],
                reason_code: fidl_ieee80211::ReasonCode::UnspecifiedReason,
                locally_initiated: false,
            },
        };
        let state = state.on_mlme_event(deauth_ind, &mut h.context);
        expect_result(
            receiver_one,
            ConnectResult::Failed(ConnectFailure::AuthenticationFailure(
                fidl_mlme::AuthenticateResultCode::Refused,
            )),
        );
        assert_idle(state);
    }

    #[test]
    fn deauth_while_associating() {
        let mut h = TestHelper::new();
        let (cmd_one, receiver_one) = connect_command_one();
        let bss_protection = cmd_one.bss.protection();
        let state = associating_state(cmd_one);
        let deauth_ind = MlmeEvent::DeauthenticateInd {
            ind: fidl_mlme::DeauthenticateIndication {
                peer_sta_address: [7, 7, 7, 7, 7, 7],
                reason_code: fidl_ieee80211::ReasonCode::UnspecifiedReason,
                locally_initiated: false,
            },
        };
        let state = state.on_mlme_event(deauth_ind, &mut h.context);
        expect_result(
            receiver_one,
            ConnectResult::Failed(
                AssociationFailure {
                    bss_protection,
                    code: fidl_mlme::AssociateResultCode::RefusedReasonUnspecified,
                }
                .into(),
            ),
        );
        assert_idle(state);
    }

    #[test]
    fn disassoc_while_associating() {
        let mut h = TestHelper::new();
        let (cmd_one, receiver_one) = connect_command_one();
        let bss_protection = cmd_one.bss.protection();
        let state = associating_state(cmd_one);
        let disassoc_ind = MlmeEvent::DisassociateInd {
            ind: fidl_mlme::DisassociateIndication {
                peer_sta_address: [7, 7, 7, 7, 7, 7],
                reason_code: fidl_ieee80211::ReasonCode::PeerkeyMismatch,
                locally_initiated: false,
            },
        };
        let state = state.on_mlme_event(disassoc_ind, &mut h.context);
        expect_result(
            receiver_one,
            ConnectResult::Failed(
                AssociationFailure {
                    bss_protection,
                    code: fidl_mlme::AssociateResultCode::RefusedReasonUnspecified,
                }
                .into(),
            ),
        );
        assert_idle(state);
    }

    #[test]
    fn supplicant_fails_to_start_while_associating() {
        let mut h = TestHelper::new();
        let (supplicant, suppl_mock) = mock_psk_supplicant();
        let (command, receiver) = connect_command_wpa2(supplicant);
        let bssid = command.bss.bssid.clone();
        let state = associating_state(command);

        suppl_mock.set_start_failure(format_err!("failed to start supplicant"));

        // (mlme->sme) Send an AssociateConf
        let assoc_conf = create_assoc_conf(fidl_mlme::AssociateResultCode::Success);
        let _state = state.on_mlme_event(assoc_conf, &mut h.context);

        expect_deauth_req(&mut h.mlme_stream, bssid, fidl_ieee80211::ReasonCode::StaLeaving);
        let result: ConnectResult = EstablishRsnaFailure {
            auth_method: Some(auth::MethodName::Psk),
            reason: EstablishRsnaFailureReason::StartSupplicantFailed,
        }
        .into();
        expect_result(receiver, result.clone());
    }

    #[test]
    fn bad_eapol_frame_while_establishing_rsna() {
        let mut h = TestHelper::new();
        let (supplicant, suppl_mock) = mock_psk_supplicant();
        let (command, mut receiver) = connect_command_wpa2(supplicant);
        let bssid = command.bss.bssid.clone();
        let state = establishing_rsna_state(command);

        // doesn't matter what we mock here
        let update = SecAssocUpdate::Status(SecAssocStatus::EssSaEstablished);
        suppl_mock.set_on_eapol_frame_updates(vec![update]);

        // (mlme->sme) Send an EapolInd with bad eapol data
        let eapol_ind = create_eapol_ind(bssid.clone(), vec![1, 2, 3, 4]);
        let s = state.on_mlme_event(eapol_ind, &mut h.context);

        assert_eq!(Ok(None), receiver.try_recv());
        assert_variant!(s, ClientState::Associated(state) => {
            assert_variant!(&state.link_state, LinkState::EstablishingRsna { .. })});

        expect_stream_empty(&mut h.mlme_stream, "unexpected event in mlme stream");
        expect_stream_empty(&mut h.info_stream, "unexpected event in info stream");
    }

    #[test]
    fn supplicant_fails_to_process_eapol_while_establishing_rsna() {
        let mut h = TestHelper::new();
        let (supplicant, suppl_mock) = mock_psk_supplicant();
        let (command, mut receiver) = connect_command_wpa2(supplicant);
        let bssid = command.bss.bssid.clone();
        let state = establishing_rsna_state(command);

        suppl_mock.set_on_eapol_frame_failure(format_err!("supplicant::on_eapol_frame fails"));

        // (mlme->sme) Send an EapolInd
        let eapol_ind = create_eapol_ind(bssid.clone(), test_utils::eapol_key_frame().into());
        let s = state.on_mlme_event(eapol_ind, &mut h.context);

        assert_eq!(Ok(None), receiver.try_recv());
        assert_variant!(s, ClientState::Associated(state) => {
            assert_variant!(&state.link_state, LinkState::EstablishingRsna { .. })});

        expect_stream_empty(&mut h.mlme_stream, "unexpected event in mlme stream");
        expect_stream_empty(&mut h.info_stream, "unexpected event in info stream");
    }

    #[test]
    fn reject_foreign_eapol_frames() {
        let mut h = TestHelper::new();
        let (supplicant, mock) = mock_psk_supplicant();
        let state = link_up_state_protected(supplicant, [7; 6]);
        mock.set_on_eapol_frame_callback(|| {
            panic!("eapol frame should not have been processed");
        });

        // Send an EapolInd from foreign BSS.
        let eapol_ind = create_eapol_ind([1; 6], test_utils::eapol_key_frame().into());
        let state = state.on_mlme_event(eapol_ind, &mut h.context);

        // Verify state did not change.
        assert_variant!(state, ClientState::Associated(state) => {
            assert_variant!(
                &state.link_state,
                LinkState::LinkUp(state) => assert_variant!(&state.protection, Protection::Rsna(_))
            )
        });
    }

    #[test]
    fn wrong_password_while_establishing_rsna() {
        let mut h = TestHelper::new();
        let (supplicant, suppl_mock) = mock_psk_supplicant();
        let (command, receiver) = connect_command_wpa2(supplicant);
        let bssid = command.bss.bssid.clone();
        let state = establishing_rsna_state(command);

        // (mlme->sme) Send an EapolInd, mock supplicant with wrong password status
        let update = SecAssocUpdate::Status(SecAssocStatus::WrongPassword);
        let _state = on_eapol_ind(state, &mut h, bssid, &suppl_mock, vec![update]);

        expect_deauth_req(&mut h.mlme_stream, bssid, fidl_ieee80211::ReasonCode::StaLeaving);
        let result: ConnectResult = EstablishRsnaFailure {
            auth_method: Some(auth::MethodName::Psk),
            reason: EstablishRsnaFailureReason::InternalError,
        }
        .into();
        expect_result(receiver, result.clone());
    }

    #[test]
    fn overall_timeout_while_establishing_rsna() {
        let mut h = TestHelper::new();
        let (supplicant, _suppl_mock) = mock_psk_supplicant();
        let (command, receiver) = connect_command_wpa2(supplicant);
        let bssid = command.bss.bssid.clone();

        // Start in an "Associating" state
        let state = ClientState::from(testing::new_state(Associating {
            cfg: ClientConfig::default(),
            cmd: command,
            chan: fake_channel(),
            cap: None,
            protection_ie: None,
        }));
        let assoc_conf = create_assoc_conf(fidl_mlme::AssociateResultCode::Success);
        let state = state.on_mlme_event(assoc_conf, &mut h.context);

        let (_, timed_event) = h.time_stream.try_next().unwrap().expect("expect timed event");
        assert_variant!(timed_event.event, Event::EstablishingRsnaTimeout(..));

        expect_stream_empty(&mut h.mlme_stream, "unexpected event in mlme stream");

        let _state = state.handle_timeout(timed_event.id, timed_event.event, &mut h.context);

        expect_deauth_req(&mut h.mlme_stream, bssid, fidl_ieee80211::ReasonCode::StaLeaving);
        expect_result(
            receiver,
            EstablishRsnaFailure {
                auth_method: Some(auth::MethodName::Psk),
                reason: EstablishRsnaFailureReason::OverallTimeout,
            }
            .into(),
        );
    }

    #[test]
    fn key_frame_exchange_timeout_while_establishing_rsna() {
        let mut h = TestHelper::new();
        let (supplicant, suppl_mock) = mock_psk_supplicant();
        let (command, receiver) = connect_command_wpa2(supplicant);
        let bssid = command.bss.bssid.clone();
        let state = establishing_rsna_state(command);

        // (mlme->sme) Send an EapolInd, mock supplication with key frame
        let update = SecAssocUpdate::TxEapolKeyFrame(test_utils::eapol_key_frame());
        let mut state = on_eapol_ind(state, &mut h, bssid, &suppl_mock, vec![update]);

        for i in 1..=3 {
            println!("send eapol attempt: {}", i);
            expect_eapol_req(&mut h.mlme_stream, bssid);
            expect_stream_empty(&mut h.mlme_stream, "unexpected event in mlme stream");

            let (_, timed_event) = h.time_stream.try_next().unwrap().expect("expect timed event");
            assert_variant!(timed_event.event, Event::KeyFrameExchangeTimeout(ref event) => {
                assert_eq!(event.attempt, i)
            });
            state = state.handle_timeout(timed_event.id, timed_event.event, &mut h.context);
        }

        expect_deauth_req(&mut h.mlme_stream, bssid, fidl_ieee80211::ReasonCode::StaLeaving);
        expect_result(
            receiver,
            EstablishRsnaFailure {
                auth_method: Some(auth::MethodName::Psk),
                reason: EstablishRsnaFailureReason::KeyFrameExchangeTimeout,
            }
            .into(),
        );
    }

    #[test]
    fn gtk_rotation_during_link_up() {
        let mut h = TestHelper::new();
        let (supplicant, suppl_mock) = mock_psk_supplicant();
        let bssid = [7; 6];
        let state = link_up_state_protected(supplicant, bssid);

        // (mlme->sme) Send an EapolInd, mock supplication with key frame and GTK
        let key_frame = SecAssocUpdate::TxEapolKeyFrame(test_utils::eapol_key_frame());
        let gtk = SecAssocUpdate::Key(Key::Gtk(test_utils::gtk()));
        let mut state = on_eapol_ind(state, &mut h, bssid, &suppl_mock, vec![key_frame, gtk]);

        // EAPoL frame is sent out, but state still remains the same
        expect_eapol_req(&mut h.mlme_stream, bssid);
        expect_set_gtk(&mut h.mlme_stream);
        expect_stream_empty(&mut h.mlme_stream, "unexpected event in mlme stream");
        assert_variant!(&state, ClientState::Associated(state) => {
            assert_variant!(&state.link_state, LinkState::LinkUp { .. });
        });

        // Any timeout is ignored
        let (_, timed_event) = h.time_stream.try_next().unwrap().expect("expect timed event");
        state = state.handle_timeout(timed_event.id, timed_event.event, &mut h.context);
        assert_variant!(&state, ClientState::Associated(state) => {
            assert_variant!(&state.link_state, LinkState::LinkUp { .. });
        });
    }

    #[test]
    fn connect_while_link_up() {
        let mut h = TestHelper::new();
        let state = link_up_state(connect_command_one().0.bss);
        let state = state.connect(connect_command_two().0, &mut h.context);
        let state = exchange_deauth(state, &mut h);
        expect_join_request(&mut h.mlme_stream, connect_command_two().0.bss.bssid);
        assert_joining(state, &connect_command_two().0.bss);
    }

    #[test]
    fn disconnect_while_idle() {
        let mut h = TestHelper::new();
        let new_state = idle_state()
            .disconnect(&mut h.context, fidl_sme::UserDisconnectReason::WlanSmeUnitTesting);
        assert_idle(new_state);
        // Expect no messages to the MLME
        assert!(h.mlme_stream.try_next().is_err());
    }

    #[test]
    fn disconnect_while_joining() {
        let mut h = TestHelper::new();
        let (cmd, receiver) = connect_command_one();
        let state = joining_state(cmd);
        let state =
            state.disconnect(&mut h.context, fidl_sme::UserDisconnectReason::WlanSmeUnitTesting);
        expect_result(receiver, ConnectResult::Canceled);
        assert_idle(state);
    }

    #[test]
    fn disconnect_while_authenticating() {
        let mut h = TestHelper::new();
        let (cmd, receiver) = connect_command_one();
        let state = authenticating_state(cmd);
        let state =
            state.disconnect(&mut h.context, fidl_sme::UserDisconnectReason::WlanSmeUnitTesting);
        expect_result(receiver, ConnectResult::Canceled);
        assert_idle(state);
    }

    #[test]
    fn disconnect_while_associating() {
        let mut h = TestHelper::new();
        let (cmd, receiver) = connect_command_one();
        let state = associating_state(cmd);
        let state =
            state.disconnect(&mut h.context, fidl_sme::UserDisconnectReason::WlanSmeUnitTesting);
        let state = exchange_deauth(state, &mut h);
        expect_result(receiver, ConnectResult::Canceled);
        assert_idle(state);

        assert_inspect_tree!(h._inspector, root: contains {
            state_events: {
                // There's no disconnect_ctx node
                "0": {
                    "@time": AnyProperty,
                    ctx: AnyProperty,
                    from: AnyProperty,
                    to: AnyProperty,
                }
            }
        });
    }

    #[test]
    fn disconnect_while_link_up() {
        let mut h = TestHelper::new();
        let state = link_up_state(connect_command_one().0.bss);
        let state =
            state.disconnect(&mut h.context, fidl_sme::UserDisconnectReason::FailedToConnect);
        let state = exchange_deauth(state, &mut h);
        assert_idle(state);

        assert_inspect_tree!(h._inspector, root: contains {
            state_events: {
                "0": contains {
                    disconnect_ctx: {
                        reason_code: (1u64 << 16) + 1,
                        locally_initiated: true,
                    }
                }
            }
        });
    }

    #[test]
    fn increment_att_id_on_connect() {
        let mut h = TestHelper::new();
        let state = idle_state();
        assert_eq!(h.context.att_id, 0);

        let state = state.connect(connect_command_one().0, &mut h.context);
        assert_eq!(h.context.att_id, 1);

        let state =
            state.disconnect(&mut h.context, fidl_sme::UserDisconnectReason::WlanSmeUnitTesting);
        assert_eq!(h.context.att_id, 1);

        let state = state.connect(connect_command_two().0, &mut h.context);
        assert_eq!(h.context.att_id, 2);

        let _state = state.connect(connect_command_one().0, &mut h.context);
        assert_eq!(h.context.att_id, 3);
    }

    #[test]
    fn increment_att_id_on_disassociate_ind() {
        let mut h = TestHelper::new();
        let state = link_up_state(Box::new(fake_bss!(Open, ssid: b"bar".to_vec(), bssid: [8; 6])));
        assert_eq!(h.context.att_id, 0);

        let disassociate_ind = MlmeEvent::DisassociateInd {
            ind: fidl_mlme::DisassociateIndication {
                peer_sta_address: [0, 0, 0, 0, 0, 0],
                reason_code: fidl_ieee80211::ReasonCode::UnspecifiedReason,
                locally_initiated: false,
            },
        };

        let state = state.on_mlme_event(disassociate_ind, &mut h.context);
        assert_associating(state, &fake_bss!(Open, ssid: b"bar".to_vec(), bssid: [8; 6]));
        assert_eq!(h.context.att_id, 1);
    }

    #[test]
    fn log_disconnect_ctx_on_disassoc_from_link_up() {
        let mut h = TestHelper::new();
        let state = link_up_state(Box::new(fake_bss!(Open, ssid: b"bar".to_vec(), bssid: [8; 6])));
        assert_eq!(h.context.att_id, 0);

        let disassociate_ind = MlmeEvent::DisassociateInd {
            ind: fidl_mlme::DisassociateIndication {
                peer_sta_address: [0, 0, 0, 0, 0, 0],
                reason_code: fidl_ieee80211::ReasonCode::UnacceptablePowerCapability,
                locally_initiated: true,
            },
        };
        let state = state.on_mlme_event(disassociate_ind, &mut h.context);
        assert_associating(state, &fake_bss!(Open, ssid: b"bar".to_vec(), bssid: [8; 6]));

        assert_inspect_tree!(h._inspector, root: contains {
            state_events: {
                "0": contains {
                    disconnect_ctx: {
                        reason_code: 10u64,
                        locally_initiated: true,
                    }
                }
            }
        });
    }

    #[test]
    fn do_not_log_disconnect_ctx_on_disassoc_from_non_link_up() {
        let mut h = TestHelper::new();
        let (supplicant, _suppl_mock) = mock_psk_supplicant();
        let (command, _receiver) = connect_command_wpa2(supplicant);
        let state = establishing_rsna_state(command);

        let disassociate_ind = MlmeEvent::DisassociateInd {
            ind: fidl_mlme::DisassociateIndication {
                peer_sta_address: [0, 0, 0, 0, 0, 0],
                reason_code: fidl_ieee80211::ReasonCode::UnacceptablePowerCapability,
                locally_initiated: true,
            },
        };
        let state = state.on_mlme_event(disassociate_ind, &mut h.context);
        assert_associating(state, &fake_bss!(Wpa2, ssid: b"wpa2".to_vec()));

        assert_inspect_tree!(h._inspector, root: contains {
            state_events: {
                // There's no disconnect_ctx node
                "0": {
                    "@time": AnyProperty,
                    ctx: AnyProperty,
                    from: AnyProperty,
                    to: AnyProperty,
                }
            }
        });
    }

    #[test]
    fn connection_ping() {
        let mut h = TestHelper::new();

        let (cmd, _receiver) = connect_command_one();

        // Start in an "Associating" state
        let state = ClientState::from(testing::new_state(Associating {
            cfg: ClientConfig::default(),
            cmd,
            chan: fake_channel(),
            cap: None,
            protection_ie: None,
        }));
        let assoc_conf = create_assoc_conf(fidl_mlme::AssociateResultCode::Success);
        let state = state.on_mlme_event(assoc_conf, &mut h.context);

        // Verify ping timeout is scheduled
        let (_, timed_event) = h.time_stream.try_next().unwrap().expect("expect timed event");
        let first_ping = assert_variant!(timed_event.event.clone(), Event::ConnectionPing(info) => {
            assert_eq!(info.connected_since, info.now);
            assert!(info.last_reported.is_none());
            info
        });
        // Verify that ping is reported
        assert_variant!(h.info_stream.try_next(), Ok(Some(InfoEvent::ConnectionPing(ref info))) => {
            assert_eq!(info.connected_since, info.now);
            assert!(info.last_reported.is_none());
        });

        // Trigger the above timeout
        let _state = state.handle_timeout(timed_event.id, timed_event.event, &mut h.context);

        // Verify ping timeout is scheduled again
        let (_, timed_event) = h.time_stream.try_next().unwrap().expect("expect timed event");
        assert_variant!(timed_event.event, Event::ConnectionPing(ref info) => {
            assert_variant!(info.last_reported, Some(time) => assert_eq!(time, first_ping.now));
        });
        // Verify that ping is reported
        assert_variant!(h.info_stream.try_next(), Ok(Some(InfoEvent::ConnectionPing(ref info))) => {
            assert_variant!(info.last_reported, Some(time) => assert_eq!(time, first_ping.now));
        });
    }

    #[test]
    fn disconnect_reported_on_deauth_ind() {
        let mut h = TestHelper::new();
        let state = link_up_state(Box::new(fake_bss!(Open, ssid: b"bar".to_vec(), bssid: [8; 6])));

        let deauth_ind = MlmeEvent::DeauthenticateInd {
            ind: fidl_mlme::DeauthenticateIndication {
                peer_sta_address: [0, 0, 0, 0, 0, 0],
                reason_code: fidl_ieee80211::ReasonCode::LeavingNetworkDeauth,
                locally_initiated: true,
            },
        };

        let _state = state.on_mlme_event(deauth_ind, &mut h.context);
        assert_variant!(h.info_stream.try_next(), Ok(Some(InfoEvent::DisconnectInfo(info))) => {
            assert_eq!(info.last_rssi, 60);
            assert_eq!(info.last_snr, 30);
            assert_eq!(info.ssid, b"bar");
            assert_eq!(info.wsc, None);
            assert_eq!(info.protection, BssProtection::Open);
            assert_eq!(info.bssid, [8; 6]);
            assert_variant!(info.disconnect_source, DisconnectSource::Mlme(DisconnectCause {
                mlme_event_name: DisconnectMlmeEventName::DeauthenticateIndication,
                reason_code: fidl_ieee80211::ReasonCode::LeavingNetworkDeauth,
            }));
        });
    }

    #[test]
    fn disconnect_reported_on_disassoc_ind() {
        let mut h = TestHelper::new();
        let state = link_up_state(Box::new(fake_bss!(Open, ssid: b"bar".to_vec(), bssid: [8; 6])));

        let deauth_ind = MlmeEvent::DisassociateInd {
            ind: fidl_mlme::DisassociateIndication {
                peer_sta_address: [0, 0, 0, 0, 0, 0],
                reason_code: fidl_ieee80211::ReasonCode::ReasonInactivity,
                locally_initiated: true,
            },
        };

        let _state = state.on_mlme_event(deauth_ind, &mut h.context);
        assert_variant!(h.info_stream.try_next(), Ok(Some(InfoEvent::DisconnectInfo(info))) => {
            assert_eq!(info.last_rssi, 60);
            assert_eq!(info.last_snr, 30);
            assert_eq!(info.ssid, b"bar");
            assert_eq!(info.wsc, None);
            assert_eq!(info.protection, BssProtection::Open);
            assert_eq!(info.bssid, [8; 6]);
            assert_variant!(info.disconnect_source, DisconnectSource::Mlme(DisconnectCause {
                mlme_event_name: DisconnectMlmeEventName::DisassociateIndication,
                reason_code: fidl_ieee80211::ReasonCode::ReasonInactivity,
            }));
        });
    }

    #[test]
    fn disconnect_reported_on_manual_disconnect() {
        let mut h = TestHelper::new();
        let state = link_up_state(Box::new(fake_bss!(Open, ssid: b"bar".to_vec(), bssid: [8; 6])));

        let _state =
            state.disconnect(&mut h.context, fidl_sme::UserDisconnectReason::WlanSmeUnitTesting);
        assert_variant!(h.info_stream.try_next(), Ok(Some(InfoEvent::DisconnectInfo(info))) => {
            assert_eq!(info.last_rssi, 60);
            assert_eq!(info.last_snr, 30);
            assert_eq!(info.ssid, b"bar");
            assert_eq!(info.wsc, None);
            assert_eq!(info.protection, BssProtection::Open);
            assert_eq!(info.bssid, [8; 6]);
            assert_eq!(info.disconnect_source, DisconnectSource::User(fidl_sme::UserDisconnectReason::WlanSmeUnitTesting));
        });
    }

    #[test]
    fn disconnect_reported_on_manual_disconnect_with_wsc() {
        let mut h = TestHelper::new();
        let bss = fake_bss!(Open, ssid: b"bar".to_vec(), bssid: [8; 6], ies_overrides: IesOverrides::new().set_raw(
            get_vendor_ie_bytes_for_wsc_ie(&fake_probe_resp_wsc_ie_bytes()).expect("getting vendor ie bytes")
        ));
        println!("{:02x?}", bss);

        let state = link_up_state(Box::new(bss));

        let _state =
            state.disconnect(&mut h.context, fidl_sme::UserDisconnectReason::WlanSmeUnitTesting);
        assert_variant!(h.info_stream.try_next(), Ok(Some(InfoEvent::DisconnectInfo(info))) => {
            assert_eq!(info.last_rssi, 60);
            assert_eq!(info.last_snr, 30);
            assert_eq!(info.ssid, b"bar");
            assert_eq!(info.wsc, Some(fake_probe_resp_wsc_ie()));
            assert_eq!(info.protection, BssProtection::Open);
            assert_eq!(info.bssid, [8; 6]);
            assert_eq!(info.disconnect_source, DisconnectSource::User(fidl_sme::UserDisconnectReason::WlanSmeUnitTesting));
        });
    }

    #[test]
    fn bss_channel_switch_ind() {
        let mut h = TestHelper::new();
        let state = link_up_state(Box::new(fake_bss!(Open,
                                                     ssid: b"bar".to_vec(),
                                                     bssid: [8; 6],
                                                     chan: fidl_common::WlanChan {
                                                         primary: 1,
                                                         secondary80: 0,
                                                         cbw: fidl_common::Cbw::Cbw20
                                                     }
        )));

        let switch_ind =
            MlmeEvent::OnChannelSwitched { info: fidl_mlme::ChannelSwitchInfo { new_channel: 36 } };

        assert_variant!(&state, ClientState::Associated(state) => {
            assert_eq!(state.bss.chan.primary, 1);
        });
        let state = state.on_mlme_event(switch_ind, &mut h.context);
        assert_variant!(state, ClientState::Associated(state) => {
            assert_eq!(state.bss.chan.primary, 36);
        });
    }

    #[test]
    fn join_failure_capabilities_incompatible_softmac() {
        let (mut command, _receiver) = connect_command_one();
        command.bss = Box::new(fake_bss!(Open,
            ssid: b"foo".to_vec(),
            bssid: [7, 7, 7, 7, 7, 7],
            // Set a fake basic rate that our mocked client can't support, causing
            // `derive_join_and_capabilities` to fail, which in turn fails the join.
            ies_overrides: IesOverrides::new()
                .set(ie::IeType::SUPPORTED_RATES, vec![0xff])
        ));

        let mut h = TestHelper::new();
        let state = idle_state().connect(command, &mut h.context);

        // State did not change to Joining because the command was ignored due to incompatibility.
        assert_variant!(state, ClientState::Idle(_));
    }

    #[test]
    fn join_failure_capabilities_incompatible_fullmac() {
        let (mut command, _receiver) = connect_command_one();
        command.bss = Box::new(fake_bss!(Open,
            ssid: b"foo".to_vec(),
            bssid: [7, 7, 7, 7, 7, 7],
            // Set a fake basic rate that our mocked client can't support, causing
            // `derive_join_and_capabilities` to fail, which in turn fails the join.
            ies_overrides: IesOverrides::new()
                .set(ie::IeType::SUPPORTED_RATES, vec![0xff])
        ));

        let mut h = TestHelper::new();
        // set as full mac
        h.context.is_softmac = false;

        let state = idle_state().connect(command, &mut h.context);

        // State did not change to Joining because the command was ignored due to incompatibility.
        assert_variant!(state, ClientState::Idle(_));
    }

    #[test]
    fn join_success_softmac() {
        let (command, _receiver) = connect_command_one();
        let mut h = TestHelper::new();
        let state = idle_state().connect(command, &mut h.context);

        // State changed to Joining, capabilities preserved.
        let cap = assert_variant!(&state, ClientState::Joining(state) => &state.cap);
        assert!(cap.is_some());
    }

    #[test]
    fn join_success_fullmac() {
        let (command, _receiver) = connect_command_one();
        let mut h = TestHelper::new();
        // set full mac
        h.context.is_softmac = false;
        let state = idle_state().connect(command, &mut h.context);

        // State changed to Joining, capabilities discarded as FullMAC ignore them anyway.
        let cap = assert_variant!(&state, ClientState::Joining(state) => &state.cap);
        assert!(cap.is_none());
    }

    #[test]
    fn join_failure_rsne_wrapped_in_legacy_wpa() {
        let (supplicant, _suppl_mock) = mock_psk_supplicant();

        let (mut command, _receiver) = connect_command_wpa2(supplicant);
        // Take the RSNA and wrap it in LegacyWpa to make it invalid.
        if let Protection::Rsna(rsna) = command.protection {
            command.protection = Protection::LegacyWpa(rsna);
        } else {
            panic!("command is guaranteed to be contain legacy wpa");
        };

        let mut h = TestHelper::new();
        let state = idle_state().connect(command, &mut h.context);

        // State did not change to Joining because command is invalid, thus ignored.
        assert_variant!(state, ClientState::Idle(_));
    }

    #[test]
    fn join_failure_legacy_wpa_wrapped_in_rsna() {
        let (supplicant, _suppl_mock) = mock_psk_supplicant();

        let (mut command, _receiver) = connect_command_wpa1(supplicant);
        // Take the LegacyWpa RSNA and wrap it in Rsna to make it invalid.
        if let Protection::LegacyWpa(rsna) = command.protection {
            command.protection = Protection::Rsna(rsna);
        } else {
            panic!("command is guaranteed to be contain legacy wpa");
        };

        let mut h = TestHelper::new();
        let state = idle_state();
        let state = state.connect(command, &mut h.context);

        // State did not change to Joining because command is invalid, thus ignored.
        assert_variant!(state, ClientState::Idle(_));
    }

    #[test]
    fn fill_wmm_ie_associating() {
        let mut h = TestHelper::new();
        let (cmd, _receiver) = connect_command_one();
        let resp = fidl_mlme::AssociateConfirm {
            result_code: fidl_mlme::AssociateResultCode::Success,
            association_id: 1,
            cap_info: 0,
            rates: vec![0x0c, 0x12, 0x18, 0x24, 0x30, 0x48, 0x60, 0x6c],
            ht_cap: cmd.bss.raw_ht_cap().map(Box::new),
            vht_cap: cmd.bss.raw_vht_cap().map(Box::new),
            wmm_param: Some(Box::new(fake_wmm_param())),
        };

        let state = associating_state(cmd);
        let state = state.on_mlme_event(MlmeEvent::AssociateConf { resp }, &mut h.context);
        assert_variant!(state, ClientState::Associated(state) => {
            assert!(state.wmm_param.is_some());
        });
    }

    #[test]
    fn status_returns_last_rssi_snr() {
        let mut h = TestHelper::new();
        let time_a = now();

        let state =
            link_up_state(Box::new(fake_bss!(Open, ssid: b"RSSI".to_vec(), bssid: [42; 6])));
        let state = state.on_mlme_event(signal_report_with_rssi_snr(-42, 20), &mut h.context);
        assert_eq!(state.status().connected_to.unwrap().rssi_dbm, -42);
        assert_eq!(state.status().connected_to.unwrap().snr_db, 20);
        assert!(state.status().connected_to.unwrap().signal_report_time > time_a);

        let time_b = now();
        assert!(state.status().connected_to.unwrap().signal_report_time < time_b);

        let state = state.on_mlme_event(signal_report_with_rssi_snr(-24, 10), &mut h.context);
        assert_eq!(state.status().connected_to.unwrap().rssi_dbm, -24);
        assert_eq!(state.status().connected_to.unwrap().snr_db, 10);
        assert!(state.status().connected_to.unwrap().signal_report_time > time_b);

        let time_c = now();
        assert!(state.status().connected_to.unwrap().signal_report_time < time_c);
    }

    fn test_sae_frame_rx_tx(
        mock_supplicant_controller: MockSupplicantController,
        state: ClientState,
    ) -> ClientState {
        let mut h = TestHelper::new();
        let frame_rx = fidl_mlme::SaeFrame {
            peer_sta_address: [0xaa; 6],
            status_code: fidl_ieee80211::StatusCode::Success,
            seq_num: 1,
            sae_fields: vec![1, 2, 3, 4, 5],
        };
        let frame_tx = fidl_mlme::SaeFrame {
            peer_sta_address: [0xbb; 6],
            status_code: fidl_ieee80211::StatusCode::Success,
            seq_num: 2,
            sae_fields: vec![1, 2, 3, 4, 5, 6, 7, 8],
        };
        mock_supplicant_controller
            .set_on_sae_frame_rx_updates(vec![SecAssocUpdate::TxSaeFrame(frame_tx)]);
        let state =
            state.on_mlme_event(MlmeEvent::OnSaeFrameRx { frame: frame_rx }, &mut h.context);
        assert_variant!(h.mlme_stream.try_next(), Ok(Some(MlmeRequest::SaeFrameTx(_))));
        state
    }

    #[test]
    fn sae_sends_frame_in_authenticating() {
        let (supplicant, suppl_mock) = mock_psk_supplicant();
        let (cmd, _recv) = connect_command_wpa3(supplicant);
        let state = authenticating_state(cmd);
        let end_state = test_sae_frame_rx_tx(suppl_mock, state);
        assert_variant!(end_state, ClientState::Authenticating(_))
    }

    #[test]
    fn sae_sends_frame_in_associating() {
        let (supplicant, suppl_mock) = mock_psk_supplicant();
        let (cmd, _recv) = connect_command_wpa3(supplicant);
        let state = associating_state(cmd);
        let end_state = test_sae_frame_rx_tx(suppl_mock, state);
        assert_variant!(end_state, ClientState::Associating(_))
    }

    fn test_sae_frame_ind_resp(
        mock_supplicant_controller: MockSupplicantController,
        state: ClientState,
    ) -> ClientState {
        let mut h = TestHelper::new();
        let ind = fidl_mlme::SaeHandshakeIndication { peer_sta_address: [0xaa; 6] };
        // For the purposes of the test, skip the rx/tx and just say we succeeded.
        mock_supplicant_controller.set_on_sae_handshake_ind_updates(vec![
            SecAssocUpdate::SaeAuthStatus(AuthStatus::Success),
        ]);
        let state = state.on_mlme_event(MlmeEvent::OnSaeHandshakeInd { ind }, &mut h.context);

        let resp = assert_variant!(
            h.mlme_stream.try_next(),
            Ok(Some(MlmeRequest::SaeHandshakeResp(resp))) => resp);
        assert_eq!(resp.status_code, fidl_ieee80211::StatusCode::Success);
        state
    }

    #[test]
    fn sae_ind_in_authenticating() {
        let (supplicant, suppl_mock) = mock_psk_supplicant();
        let (cmd, _recv) = connect_command_wpa3(supplicant);
        let state = authenticating_state(cmd);
        let end_state = test_sae_frame_ind_resp(suppl_mock, state);
        assert_variant!(end_state, ClientState::Authenticating(_))
    }

    #[test]
    fn sae_ind_in_associating() {
        let (supplicant, suppl_mock) = mock_psk_supplicant();
        let (cmd, _recv) = connect_command_wpa3(supplicant);
        let state = associating_state(cmd);
        let end_state = test_sae_frame_ind_resp(suppl_mock, state);
        assert_variant!(end_state, ClientState::Associating(_))
    }

    fn test_sae_timeout(
        mock_supplicant_controller: MockSupplicantController,
        state: ClientState,
    ) -> ClientState {
        let mut h = TestHelper::new();
        let frame_tx = fidl_mlme::SaeFrame {
            peer_sta_address: [0xbb; 6],
            status_code: fidl_ieee80211::StatusCode::Success,
            seq_num: 2,
            sae_fields: vec![1, 2, 3, 4, 5, 6, 7, 8],
        };
        mock_supplicant_controller
            .set_on_sae_timeout_updates(vec![SecAssocUpdate::TxSaeFrame(frame_tx)]);
        let state = state.handle_timeout(1, event::SaeTimeout(2).into(), &mut h.context);
        assert_variant!(h.mlme_stream.try_next(), Ok(Some(MlmeRequest::SaeFrameTx(_))));
        state
    }

    fn test_sae_timeout_failure(
        mock_supplicant_controller: MockSupplicantController,
        state: ClientState,
    ) {
        let mut h = TestHelper::new();
        mock_supplicant_controller
            .set_on_sae_timeout_failure(anyhow::anyhow!("Failed to process timeout"));
        let state = state.handle_timeout(1, event::SaeTimeout(2).into(), &mut h.context);
        assert_variant!(state, ClientState::Idle(_))
    }

    #[test]
    fn sae_timeout_in_authenticating() {
        let (supplicant, suppl_mock) = mock_psk_supplicant();
        let (cmd, _recv) = connect_command_wpa3(supplicant);
        let state = authenticating_state(cmd);
        let end_state = test_sae_timeout(suppl_mock, state);
        assert_variant!(end_state, ClientState::Authenticating(_));
    }

    #[test]
    fn sae_timeout_in_associating() {
        let (supplicant, suppl_mock) = mock_psk_supplicant();
        let (cmd, _recv) = connect_command_wpa3(supplicant);
        let state = associating_state(cmd);
        let end_state = test_sae_timeout(suppl_mock, state);
        assert_variant!(end_state, ClientState::Associating(_));
    }

    #[test]
    fn sae_timeout_failure_in_authenticating() {
        let (supplicant, suppl_mock) = mock_psk_supplicant();
        let (cmd, _recv) = connect_command_wpa3(supplicant);
        let state = authenticating_state(cmd);
        test_sae_timeout_failure(suppl_mock, state);
    }

    #[test]
    fn sae_timeout_failure_in_associating() {
        let (supplicant, suppl_mock) = mock_psk_supplicant();
        let (cmd, _recv) = connect_command_wpa3(supplicant);
        let state = associating_state(cmd);
        test_sae_timeout_failure(suppl_mock, state);
    }

    #[test]
    fn update_wmm_ac_params_new() {
        let mut h = TestHelper::new();
        let wmm_param = None;
        let state = link_up_state_with_wmm(
            Box::new(fake_bss!(Open, ssid: b"wmmssid".to_vec(), bssid: [42; 6])),
            wmm_param,
        );

        let state = state.on_mlme_event(create_on_wmm_status_resp(zx::sys::ZX_OK), &mut h.context);
        assert_variant!(state, ClientState::Associated(state) => {
            assert_variant!(state.wmm_param, Some(wmm_param) => {
                assert!(wmm_param.wmm_info.ap_wmm_info().uapsd());
                assert_wmm_param_acs(&wmm_param);
            })
        });
    }

    #[test]
    fn update_wmm_ac_params_existing() {
        let mut h = TestHelper::new();

        let existing_wmm_param =
            *ie::parse_wmm_param(&fake_wmm_param().bytes[..]).expect("parse wmm");
        existing_wmm_param.wmm_info.ap_wmm_info().set_uapsd(false);
        let state = link_up_state_with_wmm(
            Box::new(fake_bss!(Open, ssid: b"wmmssid".to_vec(), bssid: [42; 6])),
            Some(existing_wmm_param),
        );

        let state = state.on_mlme_event(create_on_wmm_status_resp(zx::sys::ZX_OK), &mut h.context);
        assert_variant!(state, ClientState::Associated(state) => {
            assert_variant!(state.wmm_param, Some(wmm_param) => {
                assert!(wmm_param.wmm_info.ap_wmm_info().uapsd());
                assert_wmm_param_acs(&wmm_param);
            })
        });
    }

    #[test]
    fn update_wmm_ac_params_fails() {
        let mut h = TestHelper::new();

        let existing_wmm_param =
            *ie::parse_wmm_param(&fake_wmm_param().bytes[..]).expect("parse wmm");
        let state = link_up_state_with_wmm(
            Box::new(fake_bss!(Open, ssid: b"wmmssid".to_vec(), bssid: [42; 6])),
            Some(existing_wmm_param),
        );

        let state = state
            .on_mlme_event(create_on_wmm_status_resp(zx::sys::ZX_ERR_UNAVAILABLE), &mut h.context);
        assert_variant!(state, ClientState::Associated(state) => {
            assert_variant!(state.wmm_param, Some(wmm_param) => {
                assert_eq!(wmm_param, existing_wmm_param);
            })
        });
    }

    fn assert_wmm_param_acs(wmm_param: &ie::WmmParam) {
        assert_eq!(wmm_param.ac_be_params.aci_aifsn.aifsn(), 1);
        assert!(!wmm_param.ac_be_params.aci_aifsn.acm());
        assert_eq!(wmm_param.ac_be_params.ecw_min_max.ecw_min(), 2);
        assert_eq!(wmm_param.ac_be_params.ecw_min_max.ecw_max(), 3);
        assert_eq!({ wmm_param.ac_be_params.txop_limit }, 4);

        assert_eq!(wmm_param.ac_bk_params.aci_aifsn.aifsn(), 5);
        assert!(!wmm_param.ac_bk_params.aci_aifsn.acm());
        assert_eq!(wmm_param.ac_bk_params.ecw_min_max.ecw_min(), 6);
        assert_eq!(wmm_param.ac_bk_params.ecw_min_max.ecw_max(), 7);
        assert_eq!({ wmm_param.ac_bk_params.txop_limit }, 8);

        assert_eq!(wmm_param.ac_vi_params.aci_aifsn.aifsn(), 9);
        assert!(wmm_param.ac_vi_params.aci_aifsn.acm());
        assert_eq!(wmm_param.ac_vi_params.ecw_min_max.ecw_min(), 10);
        assert_eq!(wmm_param.ac_vi_params.ecw_min_max.ecw_max(), 11);
        assert_eq!({ wmm_param.ac_vi_params.txop_limit }, 12);

        assert_eq!(wmm_param.ac_vo_params.aci_aifsn.aifsn(), 13);
        assert!(wmm_param.ac_vo_params.aci_aifsn.acm());
        assert_eq!(wmm_param.ac_vo_params.ecw_min_max.ecw_min(), 14);
        assert_eq!(wmm_param.ac_vo_params.ecw_min_max.ecw_max(), 15);
        assert_eq!({ wmm_param.ac_vo_params.txop_limit }, 16);
    }

    // Helper functions and data structures for tests
    struct TestHelper {
        mlme_stream: MlmeStream,
        info_stream: InfoStream,
        time_stream: TimeStream,
        context: Context,
        // Inspector is kept so that root node doesn't automatically get removed from VMO
        _inspector: Inspector,
    }

    impl TestHelper {
        fn new() -> Self {
            let (mlme_sink, mlme_stream) = mpsc::unbounded();
            let (info_sink, info_stream) = mpsc::unbounded();
            let (timer, time_stream) = timer::create_timer();
            let inspector = Inspector::new();
            let hasher = WlanHasher::new([88, 77, 66, 55, 44, 33, 22, 11]);
            let context = Context {
                device_info: Arc::new(fake_device_info()),
                mlme_sink: MlmeSink::new(mlme_sink),
                timer,
                att_id: 0,
                inspect: Arc::new(inspect::SmeTree::new(inspector.root(), hasher)),
                info: InfoReporter::new(InfoSink::new(info_sink)),
                is_softmac: true,
            };
            TestHelper { mlme_stream, info_stream, time_stream, context, _inspector: inspector }
        }
    }

    fn on_eapol_ind(
        state: ClientState,
        helper: &mut TestHelper,
        bssid: [u8; 6],
        suppl_mock: &MockSupplicantController,
        update_sink: UpdateSink,
    ) -> ClientState {
        suppl_mock.set_on_eapol_frame_updates(update_sink);
        // (mlme->sme) Send an EapolInd
        let eapol_ind = create_eapol_ind(bssid.clone(), test_utils::eapol_key_frame().into());
        state.on_mlme_event(eapol_ind, &mut helper.context)
    }

    fn create_eapol_ind(bssid: [u8; 6], data: Vec<u8>) -> MlmeEvent {
        MlmeEvent::EapolInd {
            ind: fidl_mlme::EapolIndication {
                src_addr: bssid,
                dst_addr: fake_device_info().mac_addr,
                data,
            },
        }
    }

    fn exchange_deauth(state: ClientState, h: &mut TestHelper) -> ClientState {
        // (sme->mlme) Expect a DeauthenticateRequest
        assert_variant!(h.mlme_stream.try_next(), Ok(Some(MlmeRequest::Deauthenticate(req))) => {
            assert_eq!(connect_command_one().0.bss.bssid, req.peer_sta_address);
        });

        // (mlme->sme) Send a DeauthenticateConf as a response
        let deauth_conf = MlmeEvent::DeauthenticateConf {
            resp: fidl_mlme::DeauthenticateConfirm {
                peer_sta_address: connect_command_one().0.bss.bssid,
            },
        };
        state.on_mlme_event(deauth_conf, &mut h.context)
    }

    fn expect_join_request(mlme_stream: &mut MlmeStream, bssid: [u8; 6]) {
        // (sme->mlme) Expect a JoinRequest
        assert_variant!(mlme_stream.try_next(), Ok(Some(MlmeRequest::Join(req))) => {
            assert_eq!(bssid, req.selected_bss.bssid)
        });
    }

    fn expect_set_ctrl_port(
        mlme_stream: &mut MlmeStream,
        bssid: [u8; 6],
        state: fidl_mlme::ControlledPortState,
    ) {
        assert_variant!(mlme_stream.try_next(), Ok(Some(MlmeRequest::SetCtrlPort(req))) => {
            assert_eq!(req.peer_sta_address, bssid);
            assert_eq!(req.state, state);
        });
    }

    fn expect_auth_req(mlme_stream: &mut MlmeStream, bssid: [u8; 6]) {
        // (sme->mlme) Expect an AuthenticateRequest
        assert_variant!(mlme_stream.try_next(), Ok(Some(MlmeRequest::Authenticate(req))) => {
            assert_eq!(bssid, req.peer_sta_address)
        });
    }

    fn expect_deauth_req(
        mlme_stream: &mut MlmeStream,
        bssid: [u8; 6],
        reason_code: fidl_ieee80211::ReasonCode,
    ) {
        // (sme->mlme) Expect a DeauthenticateRequest
        assert_variant!(mlme_stream.try_next(), Ok(Some(MlmeRequest::Deauthenticate(req))) => {
            assert_eq!(bssid, req.peer_sta_address);
            assert_eq!(reason_code, req.reason_code);
        });
    }

    fn expect_assoc_req(mlme_stream: &mut MlmeStream, bssid: [u8; 6]) {
        assert_variant!(mlme_stream.try_next(), Ok(Some(MlmeRequest::Associate(req))) => {
            assert_eq!(bssid, req.peer_sta_address);
        });
    }

    fn expect_finalize_association_req(
        mlme_stream: &mut MlmeStream,
        chan_and_cap: (Channel, ClientCapabilities),
    ) {
        let (chan, client_cap) = chan_and_cap;
        assert_variant!(mlme_stream.try_next(), Ok(Some(MlmeRequest::FinalizeAssociation(cap))) => {
            assert_eq!(cap, client_cap.0.to_fidl_negotiated_capabilities(&chan));
        });
    }

    fn expect_eapol_req(mlme_stream: &mut MlmeStream, bssid: [u8; 6]) {
        assert_variant!(mlme_stream.try_next(), Ok(Some(MlmeRequest::Eapol(req))) => {
            assert_eq!(req.src_addr, fake_device_info().mac_addr);
            assert_eq!(req.dst_addr, bssid);
            assert_eq!(req.data, Vec::<u8>::from(test_utils::eapol_key_frame()));
        });
    }

    fn expect_set_ptk(mlme_stream: &mut MlmeStream, bssid: [u8; 6]) {
        assert_variant!(mlme_stream.try_next(), Ok(Some(MlmeRequest::SetKeys(set_keys_req))) => {
            assert_eq!(set_keys_req.keylist.len(), 1);
            let k = set_keys_req.keylist.get(0).expect("expect key descriptor");
            assert_eq!(k.key, vec![0xCCu8; test_utils::cipher().tk_bytes().unwrap()]);
            assert_eq!(k.key_id, 0);
            assert_eq!(k.key_type, fidl_mlme::KeyType::Pairwise);
            assert_eq!(k.address, bssid);
            assert_eq!(k.rsc, 0);
            assert_eq!(k.cipher_suite_oui, [0x00, 0x0F, 0xAC]);
            assert_eq!(k.cipher_suite_type, 4);
        });
    }

    fn expect_set_gtk(mlme_stream: &mut MlmeStream) {
        assert_variant!(mlme_stream.try_next(), Ok(Some(MlmeRequest::SetKeys(set_keys_req))) => {
            assert_eq!(set_keys_req.keylist.len(), 1);
            let k = set_keys_req.keylist.get(0).expect("expect key descriptor");
            assert_eq!(k.key, test_utils::gtk_bytes());
            assert_eq!(k.key_id, 2);
            assert_eq!(k.key_type, fidl_mlme::KeyType::Group);
            assert_eq!(k.address, [0xFFu8; 6]);
            assert_eq!(k.rsc, 0);
            assert_eq!(k.cipher_suite_oui, [0x00, 0x0F, 0xAC]);
            assert_eq!(k.cipher_suite_type, 4);
        });
    }

    fn expect_set_wpa1_ptk(mlme_stream: &mut MlmeStream, bssid: [u8; 6]) {
        assert_variant!(mlme_stream.try_next(), Ok(Some(MlmeRequest::SetKeys(set_keys_req))) => {
            assert_eq!(set_keys_req.keylist.len(), 1);
            let k = set_keys_req.keylist.get(0).expect("expect key descriptor");
            assert_eq!(k.key, vec![0xCCu8; test_utils::wpa1_cipher().tk_bytes().unwrap()]);
            assert_eq!(k.key_id, 0);
            assert_eq!(k.key_type, fidl_mlme::KeyType::Pairwise);
            assert_eq!(k.address, bssid);
            assert_eq!(k.rsc, 0);
            assert_eq!(k.cipher_suite_oui, [0x00, 0x50, 0xF2]);
            assert_eq!(k.cipher_suite_type, 2);
        });
    }

    fn expect_set_wpa1_gtk(mlme_stream: &mut MlmeStream) {
        assert_variant!(mlme_stream.try_next(), Ok(Some(MlmeRequest::SetKeys(set_keys_req))) => {
            assert_eq!(set_keys_req.keylist.len(), 1);
            let k = set_keys_req.keylist.get(0).expect("expect key descriptor");
            assert_eq!(k.key, test_utils::wpa1_gtk_bytes());
            assert_eq!(k.key_id, 2);
            assert_eq!(k.key_type, fidl_mlme::KeyType::Group);
            assert_eq!(k.address, [0xFFu8; 6]);
            assert_eq!(k.rsc, 0);
            assert_eq!(k.cipher_suite_oui, [0x00, 0x50, 0xF2]);
            assert_eq!(k.cipher_suite_type, 2);
        });
    }

    fn expect_set_wep_key(mlme_stream: &mut MlmeStream, bssid: [u8; 6], key_bytes: Vec<u8>) {
        assert_variant!(mlme_stream.try_next(), Ok(Some(MlmeRequest::SetKeys(set_keys_req))) => {
            assert_eq!(set_keys_req.keylist.len(), 1);
            let k = set_keys_req.keylist.get(0).expect("expect key descriptor");
            assert_eq!(k.key, &key_bytes[..]);
            assert_eq!(k.key_id, 0);
            assert_eq!(k.key_type, fidl_mlme::KeyType::Pairwise);
            assert_eq!(k.address, bssid);
            assert_eq!(k.rsc, 0);
            assert_eq!(k.cipher_suite_oui, [0x00, 0x0F, 0xAC]);
            assert_eq!(k.cipher_suite_type, 1);
        });
    }

    fn expect_result<T>(mut receiver: oneshot::Receiver<T>, expected_result: T)
    where
        T: PartialEq + ::std::fmt::Debug,
    {
        assert_eq!(Ok(Some(expected_result)), receiver.try_recv());
    }

    fn connect_command_one() -> (ConnectCommand, oneshot::Receiver<ConnectResult>) {
        let (responder, receiver) = Responder::new();
        let cmd = ConnectCommand {
            bss: Box::new(fake_bss!(Open, ssid: b"foo".to_vec(), bssid: [7, 7, 7, 7, 7, 7])),
            responder: Some(responder),
            protection: Protection::Open,
            radio_cfg: RadioConfig::default(),
        };
        (cmd, receiver)
    }

    fn connect_command_two() -> (ConnectCommand, oneshot::Receiver<ConnectResult>) {
        let (responder, receiver) = Responder::new();
        let cmd = ConnectCommand {
            bss: Box::new(fake_bss!(Open, ssid: b"bar".to_vec(), bssid: [8, 8, 8, 8, 8, 8])),
            responder: Some(responder),
            protection: Protection::Open,
            radio_cfg: RadioConfig::default(),
        };
        (cmd, receiver)
    }

    fn connect_command_wep() -> (ConnectCommand, oneshot::Receiver<ConnectResult>) {
        let (responder, receiver) = Responder::new();
        let cmd = ConnectCommand {
            bss: Box::new(fake_bss!(Wep, ssid: b"wep".to_vec())),
            responder: Some(responder),
            protection: Protection::Wep(wep_deprecated::Key::Bits40([3; 5])),
            radio_cfg: RadioConfig::default(),
        };
        (cmd, receiver)
    }

    fn connect_command_wpa1(
        supplicant: MockSupplicant,
    ) -> (ConnectCommand, oneshot::Receiver<ConnectResult>) {
        let (responder, receiver) = Responder::new();
        let wpa_ie = make_wpa1_ie();
        let cmd = ConnectCommand {
            bss: Box::new(fake_bss!(Wpa1, ssid: b"wpa1".to_vec())),
            responder: Some(responder),
            protection: Protection::LegacyWpa(Rsna {
                negotiated_protection: NegotiatedProtection::from_legacy_wpa(&wpa_ie)
                    .expect("invalid NegotiatedProtection"),
                supplicant: Box::new(supplicant),
            }),
            radio_cfg: RadioConfig::default(),
        };
        (cmd, receiver)
    }

    fn connect_command_wpa2(
        supplicant: MockSupplicant,
    ) -> (ConnectCommand, oneshot::Receiver<ConnectResult>) {
        let (responder, receiver) = Responder::new();
        let bss = fake_bss!(Wpa2, ssid: b"wpa2".to_vec());
        let rsne = Rsne::wpa2_rsne();
        let cmd = ConnectCommand {
            bss: Box::new(bss),
            responder: Some(responder),
            protection: Protection::Rsna(Rsna {
                negotiated_protection: NegotiatedProtection::from_rsne(&rsne)
                    .expect("invalid NegotiatedProtection"),
                supplicant: Box::new(supplicant),
            }),
            radio_cfg: RadioConfig::default(),
        };
        (cmd, receiver)
    }

    fn connect_command_wpa3(
        supplicant: MockSupplicant,
    ) -> (ConnectCommand, oneshot::Receiver<ConnectResult>) {
        let (responder, receiver) = Responder::new();
        let bss = fake_bss!(Wpa3, ssid: b"wpa3".to_vec());
        let rsne = Rsne::wpa3_rsne();
        let cmd = ConnectCommand {
            bss: Box::new(bss),
            responder: Some(responder),
            protection: Protection::Rsna(Rsna {
                negotiated_protection: NegotiatedProtection::from_rsne(&rsne)
                    .expect("invalid NegotiatedProtection"),
                supplicant: Box::new(supplicant),
            }),
            radio_cfg: RadioConfig::default(),
        };
        (cmd, receiver)
    }

    fn idle_state() -> ClientState {
        testing::new_state(Idle { cfg: ClientConfig::default() }).into()
    }

    fn assert_idle(state: ClientState) {
        assert_variant!(&state, ClientState::Idle(_));
    }

    fn joining_state(cmd: ConnectCommand) -> ClientState {
        testing::new_state(Joining {
            cfg: ClientConfig::default(),
            cmd,
            chan: fake_channel(),
            cap: None,
            protection_ie: None,
        })
        .into()
    }

    fn assert_joining(state: ClientState, bss: &BssDescription) {
        assert_variant!(&state, ClientState::Joining(joining) => {
            assert_eq!(joining.cmd.bss.as_ref(), bss);
        });
    }

    fn authenticating_state(cmd: ConnectCommand) -> ClientState {
        testing::new_state(Authenticating {
            cfg: ClientConfig::default(),
            cmd,
            chan: fake_channel(),
            cap: None,
            protection_ie: None,
        })
        .into()
    }

    fn associating_state(cmd: ConnectCommand) -> ClientState {
        testing::new_state(Associating {
            cfg: ClientConfig::default(),
            cmd,
            chan: fake_channel(),
            cap: None,
            protection_ie: None,
        })
        .into()
    }

    fn assert_associating(state: ClientState, bss: &BssDescription) {
        assert_variant!(&state, ClientState::Associating(associating) => {
            assert_eq!(associating.cmd.bss.as_ref(), bss);
        });
    }

    fn establishing_rsna_state(cmd: ConnectCommand) -> ClientState {
        let auth_method = cmd.protection.get_rsn_auth_method();
        let rsna = assert_variant!(cmd.protection, Protection::Rsna(rsna) => rsna);
        let link_state =
            testing::new_state(EstablishingRsna { rsna, rsna_timeout: None, resp_timeout: None })
                .into();
        testing::new_state(Associated {
            cfg: ClientConfig::default(),
            bss: cmd.bss,
            auth_method,
            responder: cmd.responder,
            last_rssi: 60,
            last_snr: 0,
            last_signal_report_time: zx::Time::ZERO,
            link_state,
            radio_cfg: RadioConfig::default(),
            chan: fake_channel(),
            cap: None,
            protection_ie: None,
            wmm_param: None,
            last_channel_switch_time: None,
        })
        .into()
    }

    fn link_up_state(bss: Box<BssDescription>) -> ClientState {
        link_up_state_with_wmm(bss, None)
    }

    fn link_up_state_with_wmm(
        bss: Box<BssDescription>,
        wmm_param: Option<ie::WmmParam>,
    ) -> ClientState {
        let link_state = testing::new_state(LinkUp {
            protection: Protection::Open,
            since: now(),
            ping_event: None,
        })
        .into();
        testing::new_state(Associated {
            cfg: ClientConfig::default(),
            responder: None,
            bss,
            auth_method: None,
            last_rssi: 60,
            last_snr: 30,
            last_signal_report_time: zx::Time::ZERO,
            link_state,
            radio_cfg: RadioConfig::default(),
            chan: fake_channel(),
            cap: None,
            protection_ie: None,
            wmm_param,
            last_channel_switch_time: None,
        })
        .into()
    }

    fn link_up_state_protected(supplicant: MockSupplicant, bssid: [u8; 6]) -> ClientState {
        let bss = fake_bss!(Wpa2, bssid: bssid, ssid: b"foo".to_vec());
        let rsne = Rsne::wpa2_rsne();
        let rsna = Rsna {
            negotiated_protection: NegotiatedProtection::from_rsne(&rsne)
                .expect("invalid NegotiatedProtection"),
            supplicant: Box::new(supplicant),
        };
        let protection = Protection::Rsna(rsna);
        let auth_method = protection.get_rsn_auth_method();
        let link_state =
            testing::new_state(LinkUp { protection, since: now(), ping_event: None }).into();
        testing::new_state(Associated {
            cfg: ClientConfig::default(),
            bss: Box::new(bss),
            responder: None,
            auth_method,
            last_rssi: 60,
            last_snr: 30,
            last_signal_report_time: zx::Time::ZERO,
            link_state,
            radio_cfg: RadioConfig::default(),
            chan: fake_channel(),
            cap: None,
            protection_ie: None,
            wmm_param: None,
            last_channel_switch_time: None,
        })
        .into()
    }

    fn fake_device_info() -> fidl_mlme::DeviceInfo {
        test_utils::fake_device_info([0, 1, 2, 3, 4, 5])
    }

    fn fake_channel() -> Channel {
        Channel { primary: 153, cbw: wlan_common::channel::Cbw::Cbw20 }
    }

    fn signal_report_with_rssi_snr(rssi_dbm: i8, snr_db: i8) -> MlmeEvent {
        MlmeEvent::SignalReport { ind: fidl_mlme::SignalReportIndication { rssi_dbm, snr_db } }
    }
}
