// Copyright 2021 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

package codegen

const tmplDecoderEncoderHeader = `
{{- define "DecoderEncoderHeader" -}}
// WARNING: This file is machine generated by fidlgen.

#pragma once

{{- /* Import the wire types and messaging API from the LLCPP bindings. */}}
{{ if .WireBindingsHeader }}
#include <{{ .WireBindingsHeader }}>
{{ end }}

// For ::fidl::fuzzing::DecoderEncoderImpl.
#include <lib/fidl/cpp/fuzzing/decoder_encoder.h>

namespace fuzzing {

inline constexpr ::std::array<::fidl::fuzzing::DecoderEncoderForType, {{ CountDecoderEncoders .Decls }}>
{{ range .Library }}{{ . }}_{{ end }}decoder_encoders = {
{{ range .Decls }}
{{- if Eq .Kind Kinds.Protocol -}}{{ template "ProtocolDecoderEncoders" . }}{{- end -}}
{{- if Eq .Kind Kinds.Struct }}{{ template "DecoderEncoder" . }}{{- end -}}
{{- if Eq .Kind Kinds.Table }}{{ template "DecoderEncoder" . }}{{- end -}}
{{- end }}
};

}  // namespace fuzzing
{{ end }}
`
