// Copyright 2019 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

import 'package:flutter/material.dart';
import 'package:fuchsia_logger/logger.dart';
import 'package:fuchsia_modular/module.dart' as modular;
import 'package:webview_flutter/webview_flutter.dart';
import 'package:fuchsia_webview_flutter/webview.dart';

import 'app.dart';

void main() {
  WebView.platform = FuchsiaWebView.create();
  setupLogger(name: 'Webview Mod');
  modular.Module();
  runApp(App());
}
