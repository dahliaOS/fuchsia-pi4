// Copyright 2021 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use {
    anyhow::{anyhow, Context, Error},
    blobfs_ramdisk::BlobfsRamdisk,
    fidl::endpoints::ServerEnd,
    fidl_fuchsia_io::DirectoryProxy,
    fidl_test_fidl_pkg::{Backing, ConnectError, HarnessRequest, HarnessRequestStream},
    fuchsia_async::Task,
    fuchsia_component::server::ServiceFs,
    fuchsia_pkg_testing::{Package, PackageBuilder, SystemImageBuilder},
    fuchsia_syslog::{fx_log_err, fx_log_info, fx_log_warn},
    futures::prelude::*,
    pkgfs_ramdisk::PkgfsRamdisk,
    std::convert::TryInto,
};

#[fuchsia_async::run_singlethreaded]
async fn main() -> Result<(), Error> {
    fuchsia_syslog::init_with_tags(&["pkg-harness"]).expect("can't init logger");

    main_inner().await.map_err(|err| {
        // Use anyhow to print the error chain.
        let err = anyhow!(err);
        fx_log_err!("error running pkg-harness: {:#}", err);
        err
    })
}

async fn main_inner() -> Result<(), Error> {
    fx_log_info!("starting pkg-harness");

    // Spin up a blobfs and install the test package.
    let test_package = make_test_package().await;
    let system_image_package =
        SystemImageBuilder::new().static_packages(&[&test_package]).build().await;
    let blobfs = BlobfsRamdisk::start().expect("started blobfs");
    let blobfs_root_dir = blobfs.root_dir().expect("getting blobfs root dir");
    test_package.write_to_blobfs_dir(&blobfs_root_dir);
    system_image_package.write_to_blobfs_dir(&blobfs_root_dir);

    // Spin up a pkgfs.
    let pkgfs = PkgfsRamdisk::builder()
        .blobfs(blobfs)
        .system_image_merkle(system_image_package.meta_far_merkle_root())
        .start()
        .expect("started pkgfs");

    // Open the test package.
    let pkgfs_backed_package = io_util::directory::open_directory(
        &pkgfs.root_dir_proxy().unwrap(),
        &format!("packages/{}/0", test_package.name()),
        fidl_fuchsia_io::OPEN_FLAG_DIRECTORY,
    )
    .await
    .unwrap();

    // Set up serving FIDL to expose the test package.
    enum IncomingService {
        Harness(HarnessRequestStream),
    }
    let mut fs = ServiceFs::new();
    fs.take_and_serve_directory_handle().context("while serving directory handle")?;
    fs.dir("svc").add_fidl_service(IncomingService::Harness);
    let () = fs
        .for_each_concurrent(None, move |svc| {
            match svc {
                IncomingService::Harness(stream) => Task::spawn(
                    serve_harness(stream, Clone::clone(&pkgfs_backed_package))
                        .map(|res| res.context("while serving test.fidl.pkg.Harness")),
                ),
            }
            .unwrap_or_else(|e| {
                fx_log_err!("error handling fidl connection: {:#}", anyhow!(e));
            })
        })
        .await;

    Ok(())
}

/// Serve test.fidl.pkg.Harness.
async fn serve_harness(
    mut stream: HarnessRequestStream,
    pkgfs_backed_package: DirectoryProxy,
) -> Result<(), Error> {
    while let Some(event) = stream.try_next().await.context("while pulling next event")? {
        let HarnessRequest::ConnectPackage { backing, dir, responder } = event;
        let pkg = match backing {
            Backing::Pkgfs => &pkgfs_backed_package,
            // TODO(fxbug.dev/75481): support pkgdir-backed packages.
            Backing::Pkgdir => {
                fx_log_warn!(
                    "pkgdir-backed packages are not yet supported. See fxbug.dev/75481 for tracking."
                );
                responder
                    .send(&mut Err(ConnectError::UnsupportedBacking))
                    .context("while sending failure response")?;
                continue;
            }
        };

        let () = pkg
            .clone(fidl_fuchsia_io::CLONE_FLAG_SAME_RIGHTS, ServerEnd::new(dir.into_channel()))
            .expect("clone to succeed");

        responder.send(&mut Ok(())).context("while sending success response")?;
    }
    Ok(())
}

/// Constructs a test package to be used in the integration tests.
async fn make_test_package() -> Package {
    let exceeds_max_buf_contents =
        repeat_by_n('a', (fidl_fuchsia_io::MAX_BUF + 1).try_into().unwrap());

    let contents = "contents".as_bytes();
    let mut builder = PackageBuilder::new("test-package")
        .add_resource_at("file", contents)
        .add_resource_at("dir/file", contents)
        .add_resource_at("dir/dir/file", contents)
        .add_resource_at("dir/dir/dir/file", contents)
        .add_resource_at("meta/file", contents)
        .add_resource_at("meta/dir/file", contents)
        .add_resource_at("meta/dir/dir/file", contents)
        .add_resource_at("meta/dir/dir/dir/file", contents)
        .add_resource_at("exceeds_max_buf", exceeds_max_buf_contents.as_bytes())
        .add_resource_at("meta/exceeds_max_buf", exceeds_max_buf_contents.as_bytes());

    // Make directory nodes of each kind (root dir, non-meta subdir, meta dir, meta subdir)
    // that overflow the fuchsia.io/Directory.ReadDirents buffer.
    for base in ["", "dir_overflow_readdirents/", "meta/", "meta/dir_overflow_readdirents/"] {
        // In the integration tests, we'll want to be able to test calling ReadDirents on a
        // directory. Since ReadDirents returns `MAX_BUF` bytes worth of directory entries, we need
        // to have test coverage for the "overflow" case where the directory has more than
        // `MAX_BUF` bytes worth of directory entries.
        //
        // Through math, we determine that we can achieve this overflow with 31 files whose names
        // are length `MAX_FILENAME`. Here is this math:
        /*
           ReadDirents -> vector<uint8>:MAX_BUF

           MAX_BUF = 8192

           struct dirent {
            // Describes the inode of the entry.
            uint64 ino;
            // Describes the length of the dirent name in bytes.
            uint8 size;
            // Describes the type of the entry. Aligned with the
            // POSIX d_type values. Use `DIRENT_TYPE_*` constants.
            uint8 type;
            // Unterminated name of entry.
            char name[0];
           }

           sizeof(dirent) if name is MAX_FILENAME = 255 bytes long = 8 + 1 + 1 + 255 = 265 bytes

           8192 / 265 ~= 30.9

           => 31 directory entries of maximum size will trigger overflow
        */
        for seed in ('a'..='z').chain('A'..='E') {
            builder = builder.add_resource_at(
                format!(
                    "{}{}",
                    base,
                    repeat_by_n(seed, fidl_fuchsia_io::MAX_FILENAME.try_into().unwrap())
                ),
                &b""[..],
            )
        }
    }
    builder.build().await.expect("build package")
}

fn repeat_by_n(seed: char, n: usize) -> String {
    std::iter::repeat(seed).take(n).collect()
}
