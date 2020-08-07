// Copyright 2020 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

.global zxr_tp_set;
.hidden zxr_tp_set;
.type   zxr_tp_set, %function
zxr_tp_set:
    mv tp, a1
    ret
