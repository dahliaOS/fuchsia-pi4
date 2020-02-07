// Copyright 2019 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#[cfg(test)]
use {
    crate::registry::device_storage::testing::*, crate::switchboard::base::SettingType,
    crate::EnvironmentBuilder, crate::Runtime, fidl_fuchsia_settings::DeviceMarker,
};

const ENV_NAME: &str = "settings_service_device_test_environment";

/// Tests that the FIDL calls for the device service result in appropriate commands
/// sent to the switchboard.
#[fuchsia_async::run_singlethreaded(test)]
async fn test_device() {
    let env =
        EnvironmentBuilder::new(Runtime::Nested(ENV_NAME), InMemoryStorageFactory::create_handle())
            .settings(&[SettingType::Device])
            .spawn_and_get_nested_environment()
            .await
            .unwrap();

    let device_proxy = env.connect_to_service::<DeviceMarker>().expect("connected to service");

    let settings = device_proxy.watch().await.expect("watch completed");

    // The tag could be in different formats based on whether it's a release build or not,
    // just check that it is nonempty.
    match settings.build_tag {
        Some(tag) => assert!(tag.len() > 0),
        None => panic!("Build tag not loaded from file"),
    }
}
