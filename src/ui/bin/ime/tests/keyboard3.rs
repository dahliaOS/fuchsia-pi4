// Copyright 2020 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.
#![cfg(test)]

use {
    anyhow::{Context as _, Result},
    fidl::endpoints::ServiceMarker as _,
    fidl_fuchsia_input as input, fidl_fuchsia_ui_input3 as ui_input3,
    fidl_fuchsia_ui_keyboard_focus as fidl_focus, fuchsia_async as fasync,
    fuchsia_component::client::connect_to_protocol,
    fuchsia_scenic as scenic,
    fuchsia_syslog::fx_log_debug,
    futures::FutureExt,
    futures::{
        future,
        stream::{FusedStream, StreamExt},
    },
    matches::assert_matches,
};

mod test_helpers;

fn create_key_down_event(key: input::Key, modifiers: ui_input3::Modifiers) -> ui_input3::KeyEvent {
    ui_input3::KeyEvent {
        key: Some(key),
        modifiers: Some(modifiers),
        type_: Some(ui_input3::KeyEventType::Pressed),
        ..ui_input3::KeyEvent::EMPTY
    }
}

fn create_key_up_event(key: input::Key, modifiers: ui_input3::Modifiers) -> ui_input3::KeyEvent {
    ui_input3::KeyEvent {
        key: Some(key),
        modifiers: Some(modifiers),
        type_: Some(ui_input3::KeyEventType::Released),
        ..ui_input3::KeyEvent::EMPTY
    }
}

async fn expect_key_event(
    listener: &mut ui_input3::KeyboardListenerRequestStream,
) -> ui_input3::KeyEvent {
    let listener_request = listener.next().await;
    if let Some(Ok(ui_input3::KeyboardListenerRequest::OnKeyEvent { event, responder, .. })) =
        listener_request
    {
        responder.send(ui_input3::KeyEventStatus::Handled).expect("responding from key listener");
        event
    } else {
        panic!("Expected key event, got {:?}", listener_request);
    }
}

async fn dispatch_and_expect_key_event<'a>(
    key_dispatcher: &'a test_helpers::KeySimulator<'a>,
    listener: &mut ui_input3::KeyboardListenerRequestStream,
    event: ui_input3::KeyEvent,
) -> Result<()> {
    let (was_handled, event_result) =
        future::join(key_dispatcher.dispatch(event.clone()), expect_key_event(listener)).await;

    assert_eq!(was_handled?, true);
    assert_eq!(event_result.key, event.key);
    assert_eq!(event_result.type_, event.type_);
    Ok(())
}

async fn expect_key_and_modifiers(
    listener: &mut ui_input3::KeyboardListenerRequestStream,
    key: input::Key,
    modifiers: ui_input3::Modifiers,
) {
    let event = expect_key_event(listener).await;
    assert_eq!(event.key, Some(key));
    assert_eq!(event.modifiers, Some(modifiers));
}

async fn test_disconnecting_keyboard_client_disconnects_listener_with_connections(
    focus_ctl: fidl_focus::ControllerProxy,
    key_simulator: &'_ test_helpers::KeySimulator<'_>,
    keyboard_service_client: ui_input3::KeyboardProxy,
    keyboard_service_other_client: &ui_input3::KeyboardProxy,
) -> Result<()> {
    fx_log_debug!("test_disconnecting_keyboard_client_disconnects_listener_with_connections");

    // Create fake client.
    let (listener_client_end, mut listener) =
        fidl::endpoints::create_request_stream::<ui_input3::KeyboardListenerMarker>()?;
    let view_ref = scenic::ViewRefPair::new()?.view_ref;

    keyboard_service_client
        .add_listener(&mut scenic::duplicate_view_ref(&view_ref)?, listener_client_end)
        .await
        .expect("add_listener for first client");

    // Create another fake client.
    let (other_listener_client_end, mut other_listener) =
        fidl::endpoints::create_request_stream::<ui_input3::KeyboardListenerMarker>()?;
    let other_view_ref = scenic::ViewRefPair::new()?.view_ref;

    keyboard_service_other_client
        .add_listener(&mut scenic::duplicate_view_ref(&other_view_ref)?, other_listener_client_end)
        .await
        .expect("add_listener for another client");

    // Focus second client.
    focus_ctl.notify(&mut scenic::duplicate_view_ref(&other_view_ref)?).await?;

    // Drop proxy, emulating first client disconnecting from it.
    std::mem::drop(keyboard_service_client);

    // Expect disconnected client key event listener to be disconnected as well.
    assert_matches!(listener.next().await, None);
    assert_matches!(listener.is_terminated(), true);

    // Ensure that the other client is still connected.
    let (key, modifiers) = (input::Key::A, ui_input3::Modifiers::CapsLock);
    let dispatched_event = create_key_down_event(key, modifiers);

    let (was_handled, _) = future::join(
        key_simulator.dispatch(dispatched_event),
        expect_key_and_modifiers(&mut other_listener, key, modifiers),
    )
    .await;

    assert_eq!(was_handled?, true);

    let dispatched_event = create_key_up_event(key, modifiers);
    let (was_handled, _) = future::join(
        key_simulator.dispatch(dispatched_event),
        expect_key_and_modifiers(&mut other_listener, key, modifiers),
    )
    .await;

    assert_eq!(was_handled?, true);
    Ok(())
}

/// Connects to the given discoverable service, with a readable context on error.
fn connect_to_service<P>() -> Result<P>
where
    P: fidl::endpoints::Proxy,
    P::Service: fidl::endpoints::DiscoverableService,
{
    connect_to_protocol::<P::Service>()
        .with_context(|| format!("Failed to connect to {}", P::Service::DEBUG_NAME))
}

fn connect_to_focus_controller() -> Result<fidl_focus::ControllerProxy> {
    connect_to_service::<_>()
}

fn connect_to_keyboard_service() -> Result<ui_input3::KeyboardProxy> {
    connect_to_service::<_>()
}

fn connect_to_key_event_injector() -> Result<ui_input3::KeyEventInjectorProxy> {
    connect_to_service::<_>()
}

#[fasync::run_singlethreaded(test)]
async fn test_disconnecting_keyboard_client_disconnects_listener_via_key_event_injector(
) -> Result<()> {
    fuchsia_syslog::init_with_tags(&["keyboard3_integration_test"])
        .expect("syslog init should not fail");

    let key_event_injector = connect_to_key_event_injector()?;

    let key_dispatcher =
        test_helpers::KeyEventInjectorDispatcher { key_event_injector: &key_event_injector };
    let key_simulator = test_helpers::KeySimulator::new(&key_dispatcher);

    let keyboard_service_client_a = connect_to_keyboard_service().context("client_a")?;

    let keyboard_service_client_b = connect_to_keyboard_service().context("client_b")?;

    test_disconnecting_keyboard_client_disconnects_listener_with_connections(
        connect_to_focus_controller()?,
        &key_simulator,
        // This one will be dropped as part of the test, so needs to be moved.
        keyboard_service_client_a,
        &keyboard_service_client_b,
    )
    .await
}

async fn test_sync_cancel_with_connections(
    focus_ctl: fidl_focus::ControllerProxy,
    key_simulator: &'_ test_helpers::KeySimulator<'_>,
    keyboard_service_client_a: &ui_input3::KeyboardProxy,
    keyboard_service_client_b: &ui_input3::KeyboardProxy,
) -> Result<()> {
    // Create fake client.
    let (listener_client_end_a, mut listener_a) =
        fidl::endpoints::create_request_stream::<ui_input3::KeyboardListenerMarker>()?;
    let view_ref_a = scenic::ViewRefPair::new()?.view_ref;

    keyboard_service_client_a
        .add_listener(&mut scenic::duplicate_view_ref(&view_ref_a)?, listener_client_end_a)
        .await
        .expect("add_listener for first client");

    // Create another fake client.
    let (listener_client_end_b, mut listener_b) =
        fidl::endpoints::create_request_stream::<ui_input3::KeyboardListenerMarker>()?;
    let view_ref_b = scenic::ViewRefPair::new()?.view_ref;

    keyboard_service_client_b
        .add_listener(&mut scenic::duplicate_view_ref(&view_ref_b)?, listener_client_end_b)
        .await
        .expect("add_listener for another client");

    let key1 = input::Key::A;
    let event1_press = ui_input3::KeyEvent {
        key: Some(key1),
        type_: Some(ui_input3::KeyEventType::Pressed),
        ..ui_input3::KeyEvent::EMPTY
    };
    let event1_release = ui_input3::KeyEvent {
        key: Some(key1),
        type_: Some(ui_input3::KeyEventType::Released),
        ..ui_input3::KeyEvent::EMPTY
    };

    // Focus client A.
    focus_ctl.notify(&mut scenic::duplicate_view_ref(&view_ref_a)?).await?;

    // Press the key and expect client A to receive the event.
    dispatch_and_expect_key_event(&key_simulator, &mut listener_a, event1_press).await?;

    assert!(listener_b.next().now_or_never().is_none(), "listener_b should have no events yet");

    // Focus client B.
    // Expect a cancel event for client A and a sync event for the client B.
    let (focus_result, client_a_event, client_b_event) = future::join3(
        focus_ctl.notify(&mut scenic::duplicate_view_ref(&view_ref_b)?),
        expect_key_event(&mut listener_a),
        expect_key_event(&mut listener_b),
    )
    .await;

    focus_result?;

    assert_eq!(
        ui_input3::KeyEvent {
            key: Some(input::Key::A),
            type_: Some(ui_input3::KeyEventType::Cancel),
            ..ui_input3::KeyEvent::EMPTY
        },
        client_a_event
    );

    assert_eq!(
        ui_input3::KeyEvent {
            key: Some(input::Key::A),
            type_: Some(ui_input3::KeyEventType::Sync),
            ..ui_input3::KeyEvent::EMPTY
        },
        client_b_event
    );

    // Release the key and expect client B to receive an event.
    dispatch_and_expect_key_event(&key_simulator, &mut listener_b, event1_release).await?;

    assert!(listener_a.next().now_or_never().is_none(), "listener_a should have no more events");

    // Focus client A again.
    focus_ctl.notify(&mut scenic::duplicate_view_ref(&view_ref_a)?).await?;

    assert!(
        listener_a.next().now_or_never().is_none(),
        "listener_a should have no more events after receiving focus"
    );

    Ok(())
}

#[fasync::run_singlethreaded(test)]
async fn test_sync_cancel_via_key_event_injector() -> Result<()> {
    fuchsia_syslog::init_with_tags(&["keyboard3_integration_test"])
        .expect("syslog init should not fail");

    // This test dispatches keys via KeyEventInjector.
    let key_event_injector = connect_to_key_event_injector()?;

    let key_dispatcher =
        test_helpers::KeyEventInjectorDispatcher { key_event_injector: &key_event_injector };
    let key_simulator = test_helpers::KeySimulator::new(&key_dispatcher);

    let keyboard_service_client_a = connect_to_keyboard_service().context("client_a")?;

    let keyboard_service_client_b = connect_to_keyboard_service().context("client_b")?;

    test_sync_cancel_with_connections(
        connect_to_focus_controller()?,
        &key_simulator,
        &keyboard_service_client_a,
        &keyboard_service_client_b,
    )
    .await
}
