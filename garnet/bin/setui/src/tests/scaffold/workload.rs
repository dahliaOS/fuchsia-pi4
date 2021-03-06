// Copyright 2021 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use crate::job;
use crate::service::message::{Audience, Messenger, Signature};
use crate::service::test::Payload;

use async_trait::async_trait;

/// [StubWorkload] provides a blank workload to be a placeholder in tests.
pub struct StubWorkload;

impl StubWorkload {
    pub fn new() -> Box<Self> {
        Box::new(Self {})
    }
}

#[async_trait]
impl job::work::Independent for StubWorkload {
    async fn execute(&mut self, _messenger: Messenger) {}
}

#[async_trait]
impl job::work::Sequential for StubWorkload {
    async fn execute(&mut self, _messenger: Messenger) {}
}

/// [Workload] provides a simple implementation of [Workload](job::Workload) for sending a test
/// Payload to a given target.
pub struct Workload {
    /// The payload to be delivered.
    payload: Payload,
    /// The [Signature] of the recipient to receive the payload.
    target: Signature,
}

impl Workload {
    pub fn new(payload: Payload, target: Signature) -> Box<Self> {
        Box::new(Self { payload, target })
    }
}

#[async_trait]
impl job::work::Independent for Workload {
    async fn execute(&mut self, messenger: Messenger) {
        messenger
            .message(self.payload.clone().into(), Audience::Messenger(self.target.clone()))
            .send()
            .ack();
    }
}

#[async_trait]
impl job::work::Sequential for Workload {
    async fn execute(&mut self, messenger: Messenger) {
        messenger
            .message(self.payload.clone().into(), Audience::Messenger(self.target.clone()))
            .send()
            .ack();
    }
}
