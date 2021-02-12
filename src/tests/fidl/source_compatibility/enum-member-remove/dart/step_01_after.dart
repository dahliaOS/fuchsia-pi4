// Copyright 2021 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

import 'package:fidl_fidl_test_enummemberremove/fidl_async.dart' as fidllib;

// [START contents]
fidllib.Color writer(String s) {
  if (s == 'red') {
    return fidllib.Color.red;
  } else if (s == 'blue') {
    return fidllib.Color.blue;
  } else {
    return fidllib.Color.$unknown;
  }
}

String reader(fidllib.Color color) {
  switch (color) {
    case fidllib.Color.blue:
      return 'blue';
    case fidllib.Color.red:
      return 'red';
    default:
      return '<unknown>';
  }
}
// [END contents]