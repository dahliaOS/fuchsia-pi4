// Copyright 2020 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use {
    crate::{key::Tk, key_data::kde, Error},
    mundane::bytes,
    wlan_common::ie::rsn::cipher::Cipher,
};

/// This IGTK provider does not support key rotations yet.
#[derive(Debug)]
pub struct IgtkProvider {
    key: Box<[u8]>,
    tk_bytes: usize,
    cipher: Cipher,
}

fn generate_random_igtk(len: usize) -> Box<[u8]> {
    let mut key = vec![0; len];
    bytes::rand(&mut key[..]);
    key.into_boxed_slice()
}

impl IgtkProvider {
    pub fn new(cipher: Cipher) -> Result<IgtkProvider, anyhow::Error> {
        let tk_bytes = cipher.tk_bytes().ok_or(Error::IgtkHierarchyUnsupportedCipherError)?;
        Ok(IgtkProvider { key: generate_random_igtk(tk_bytes), cipher, tk_bytes })
    }

    pub fn cipher(&self) -> Cipher {
        self.cipher
    }

    pub fn rotate_key(&mut self) {
        self.key = generate_random_igtk(self.tk_bytes);
    }

    pub fn get_igtk(&self) -> Igtk {
        Igtk { igtk: self.key.to_vec(), key_id: 0, ipn: [0u8; 6], cipher: self.cipher.clone() }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Igtk {
    pub igtk: Vec<u8>,
    pub key_id: u16,
    pub ipn: [u8; 6],
    pub cipher: Cipher,
}

impl Igtk {
    pub fn from_kde(element: kde::Igtk, cipher: Cipher) -> Self {
        Self { igtk: element.igtk, key_id: element.id, ipn: element.ipn, cipher }
    }
}

impl Tk for Igtk {
    fn tk(&self) -> &[u8] {
        &self.igtk[..]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use wlan_common::ie::rsn::suite_filter::DEFAULT_GROUP_MGMT_CIPHER;

    #[test]
    fn test_igtk_generation() {
        let mut igtk_hash_set = HashSet::new();
        let mut igtk_provider =
            IgtkProvider::new(DEFAULT_GROUP_MGMT_CIPHER).expect("failed creating IgtkProvider");

        for _ in 0..10000 {
            igtk_provider.rotate_key();
            let igtk = igtk_provider.get_igtk().tk().to_vec();
            assert!(igtk.iter().any(|&x| x != 0));
            assert!(!igtk_hash_set.contains(&igtk));
            igtk_hash_set.insert(igtk);
        }
    }
}
