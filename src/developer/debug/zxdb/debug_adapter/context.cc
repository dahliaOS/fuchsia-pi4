// Copyright 2020 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#include "context.h"

#include "src/developer/debug/shared/logging/logging.h"
#include "src/developer/debug/zxdb/client/process.h"
#include "src/developer/debug/zxdb/client/session.h"
#include "src/developer/debug/zxdb/client/setting_schema_definition.h"
#include "src/developer/debug/zxdb/client/thread.h"
#include "src/developer/debug/zxdb/debug_adapter/handlers/request_attach.h"
#include "src/developer/debug/zxdb/debug_adapter/handlers/request_breakpoint.h"
#include "src/developer/debug/zxdb/debug_adapter/handlers/request_continue.h"
#include "src/developer/debug/zxdb/debug_adapter/handlers/request_launch.h"
#include "src/developer/debug/zxdb/debug_adapter/handlers/request_next.h"
#include "src/developer/debug/zxdb/debug_adapter/handlers/request_pause.h"
#include "src/developer/debug/zxdb/debug_adapter/handlers/request_scopes.h"
#include "src/developer/debug/zxdb/debug_adapter/handlers/request_stacktrace.h"
#include "src/developer/debug/zxdb/debug_adapter/handlers/request_step_in.h"
#include "src/developer/debug/zxdb/debug_adapter/handlers/request_step_out.h"
#include "src/developer/debug/zxdb/debug_adapter/handlers/request_threads.h"
#include "src/developer/debug/zxdb/debug_adapter/handlers/request_variables.h"
#include "src/developer/debug/zxdb/debug_adapter/server.h"
#include "src/lib/fxl/memory/weak_ptr.h"

namespace zxdb {

DebugAdapterContext::DebugAdapterContext(Session* session, debug_ipc::StreamBuffer* stream)
    : session_(session), dap_(dap::Session::create()) {
  reader_ = std::make_shared<DebugAdapterReader>(stream);
  writer_ = std::make_shared<DebugAdapterWriter>(stream);

  dap_->registerHandler([this](const dap::InitializeRequest& req) {
    DEBUG_LOG(DebugAdapter) << "InitializeRequest received";
    dap::InitializeResponse response;
    response.supportsFunctionBreakpoints = false;
    response.supportsConfigurationDoneRequest = true;
    response.supportsEvaluateForHovers = false;
    if (req.supportsInvalidatedEvent) {
      this->supports_invalidate_event_ = req.supportsInvalidatedEvent.value();
    }
    if (req.supportsRunInTerminalRequest) {
      this->supports_run_in_terminal_ = req.supportsRunInTerminalRequest.value();
    }
    return response;
  });

  dap_->registerSentHandler([this](const dap::ResponseOrError<dap::InitializeResponse>& response) {
    DEBUG_LOG(DebugAdapter) << "InitializeResponse sent";
    // Set up events and handlers now. All messages should be sent only after Initialize response
    // is sent. Setting up earlier would lead to events and responses being sent before Initialize
    // request is processed.
    Init();
    dap_->send(dap::InitializedEvent());
  });

  dap_->onError([](const char* msg) { FX_LOGS(ERROR) << "dap::Session error:" << msg << "\r\n"; });

  dap_->connect(reader_, writer_);
}

DebugAdapterContext::~DebugAdapterContext() {
  if (init_done_) {
    session()->thread_observers().RemoveObserver(this);
    session()->process_observers().RemoveObserver(this);
  }
}

void DebugAdapterContext::Init() {
  // Register handlers with dap module.
  dap_->registerHandler([this](const dap::LaunchRequestZxdb& req) {
    DEBUG_LOG(DebugAdapter) << "LaunchRequest received";
    return OnRequestLaunch(this, req);
  });

  dap_->registerHandler([](const dap::SetExceptionBreakpointsRequest& req) {
    DEBUG_LOG(DebugAdapter) << "SetExceptionBreakpointsRequest received";
    dap::SetExceptionBreakpointsResponse response;
    return response;
  });

  dap_->registerHandler([this](const dap::SetBreakpointsRequest& req) {
    DEBUG_LOG(DebugAdapter) << "SetBreakpointsRequest received";
    return OnRequestBreakpoint(this, req);
  });

  dap_->registerHandler([](const dap::ConfigurationDoneRequest& req) {
    DEBUG_LOG(DebugAdapter) << "ConfigurationDoneRequest received";
    return dap::ConfigurationDoneResponse();
  });

  dap_->registerHandler([this](const dap::AttachRequestZxdb& req) {
    DEBUG_LOG(DebugAdapter) << "AttachRequest received";
    return OnRequestAttach(this, req);
  });

  dap_->registerHandler([this](const dap::ThreadsRequest& req) {
    DEBUG_LOG(DebugAdapter) << "ThreadRequest received";
    return OnRequestThreads(this, req);
  });

  dap_->registerHandler(
      [this](const dap::PauseRequest& req,
             std::function<void(dap::ResponseOrError<dap::PauseResponse>)> callback) {
        DEBUG_LOG(DebugAdapter) << "PauseRequest received";
        OnRequestPause(this, req, callback);
      });

  dap_->registerHandler([this](const dap::ContinueRequest& req) {
    DEBUG_LOG(DebugAdapter) << "ContinueRequest received";
    return OnRequestContinue(this, req);
  });

  dap_->registerHandler(
      [this](const dap::NextRequest& req,
             std::function<void(dap::ResponseOrError<dap::NextResponse>)> callback) {
        DEBUG_LOG(DebugAdapter) << "NextRequest received";
        OnRequestNext(this, req, callback);
      });

  dap_->registerHandler(
      [this](const dap::StepInRequest& req,
             std::function<void(dap::ResponseOrError<dap::StepInResponse>)> callback) {
        DEBUG_LOG(DebugAdapter) << "StepInRequest received";
        OnRequestStepIn(this, req, callback);
      });

  dap_->registerHandler(
      [this](const dap::StepOutRequest& req,
             std::function<void(dap::ResponseOrError<dap::StepOutResponse>)> callback) {
        DEBUG_LOG(DebugAdapter) << "StepOutRequest received";
        OnRequestStepOut(this, req, callback);
      });

  dap_->registerHandler(
      [this](const dap::StackTraceRequest& req,
             std::function<void(dap::ResponseOrError<dap::StackTraceResponse>)> callback) {
        DEBUG_LOG(DebugAdapter) << "StackTraceRequest received";
        OnRequestStackTrace(this, req, callback);
      });

  dap_->registerHandler([this](const dap::ScopesRequest& req) {
    DEBUG_LOG(DebugAdapter) << "ScopesRequest received";
    return OnRequestScopes(this, req);
  });

  dap_->registerHandler(
      [this](const dap::VariablesRequest& req,
             std::function<void(dap::ResponseOrError<dap::VariablesResponse>)> callback) {
        DEBUG_LOG(DebugAdapter) << "VariablesRequest received";
        OnRequestVariables(this, req, callback);
      });

  dap_->registerHandler([this](const dap::DisconnectRequest& req) {
    DEBUG_LOG(DebugAdapter) << "DisconnectRequest received";
    if (destroy_connection_cb_) {
      debug_ipc::MessageLoop::Current()->PostTask(
          FROM_HERE, [cb = std::move(destroy_connection_cb_)]() mutable { cb(); });
    }
    return dap::DisconnectResponse();
  });

  // Register to zxdb session events
  session()->thread_observers().AddObserver(this);
  session()->process_observers().AddObserver(this);

  init_done_ = true;
}

void DebugAdapterContext::OnStreamReadable() {
  while (auto payload = dap_->getPayload()) {
    payload();
  }
}

void DebugAdapterContext::DidCreateThread(Thread* thread) {
  dap::ThreadEvent event;
  event.reason = "started";
  event.threadId = thread->GetKoid();
  dap_->send(event);
}

void DebugAdapterContext::WillDestroyThread(Thread* thread) {
  dap::ThreadEvent event;
  event.reason = "exited";
  event.threadId = thread->GetKoid();
  dap_->send(event);
}

void DebugAdapterContext::OnThreadStopped(Thread* thread, const StopInfo& info) {
  dap::StoppedEvent event;
  switch (info.exception_type) {
    case debug_ipc::ExceptionType::kSoftwareBreakpoint:
    case debug_ipc::ExceptionType::kHardwareBreakpoint:
      event.reason = "breakpoint";
      event.description = "Breakpoint hit";
      break;
    case debug_ipc::ExceptionType::kSingleStep:
      event.reason = "step";
      break;
    case debug_ipc::ExceptionType::kPolicyError:
      event.reason = "exception";
      event.description = "Policy error";
      break;
    case debug_ipc::ExceptionType::kPageFault:
      event.reason = "exception";
      event.description = "Page fault";
      break;
    case debug_ipc::ExceptionType::kUndefinedInstruction:
      event.reason = "exception";
      event.description = "Undefined Instruction";
      break;
    case debug_ipc::ExceptionType::kUnalignedAccess:
      event.reason = "exception";
      event.description = "Unaligned Access";
      break;
    default:
      event.reason = "unknown";
  }
  event.threadId = thread->GetKoid();
  dap_->send(event);
}

void DebugAdapterContext::OnThreadFramesInvalidated(Thread* thread) {
  DeleteFrameIdsForThread(thread);
  if (supports_invalidate_event_) {
    dap::InvalidatedEvent event;
    event.threadId = thread->GetKoid();
    dap_->send(event);
  }
}

void DebugAdapterContext::DidCreateProcess(Process* process, bool autoattached_to_new_process,
                                           uint64_t timestamp) {
  dap::ProcessEvent event;
  event.name = process->GetName();
  event.isLocalProcess = false;

  switch (process->start_type()) {
    case Process::StartType::kAttach:
      event.startMethod = "attach";
      break;
    case Process::StartType::kComponent:
    case Process::StartType::kLaunch:
      event.startMethod = "launch";
      break;
  }

  bool pause_on_attach =
      session()->system().settings().GetBool(ClientSettings::System::kPauseOnAttach);
  if (autoattached_to_new_process && pause_on_attach) {
    event.startMethod = "attachForSuspendedLaunch";
  }

  dap_->send(event);
}

void DebugAdapterContext::WillDestroyProcess(Process* process, DestroyReason reason, int exit_code,
                                             uint64_t timestamp) {
  dap::ExitedEvent exit_event;            // Sent when process exits.
  dap::TerminatedEvent terminated_event;  // Sent when process is detached.
  switch (reason) {
    case ProcessObserver::DestroyReason::kExit:
      exit_event.exitCode = exit_code;
      dap_->send(exit_event);
      break;
    case ProcessObserver::DestroyReason::kDetach:
      dap_->send(terminated_event);
      break;
    case ProcessObserver::DestroyReason::kKill:
      exit_event.exitCode = -1;
      dap_->send(exit_event);
      break;
  }
}

Target* DebugAdapterContext::GetCurrentTarget() {
  auto targets = session()->system().GetTargets();
  if (targets.size() > 0) {
    // Currently debug adapter supports only one target. The default target is used to attach
    // process.
    return targets[0];
  }
  return nullptr;
}

Process* DebugAdapterContext::GetCurrentProcess() {
  auto target = GetCurrentTarget();
  if (target) {
    return target->GetProcess();
  }
  return nullptr;
}

Thread* DebugAdapterContext::GetThread(uint64_t koid) {
  Thread* match = nullptr;
  auto process = GetCurrentProcess();
  if (process) {
    auto threads = process->GetThreads();
    for (auto t : threads) {
      if (koid == t->GetKoid()) {
        match = t;
        break;
      }
    }
  }
  return match;
}

Err DebugAdapterContext::CheckStoppedThread(Thread* thread) {
  if (!thread) {
    return Err("Invalid thread.");
  }

  if (thread->GetState() != debug_ipc::ThreadRecord::State::kBlocked &&
      thread->GetState() != debug_ipc::ThreadRecord::State::kCoreDump &&
      thread->GetState() != debug_ipc::ThreadRecord::State::kSuspended) {
    return Err("Thread should be suspended but thread %llu is %s.",
               static_cast<unsigned long long>(thread->GetKoid()),
               debug_ipc::ThreadRecord::StateToString(thread->GetState()));
  }
  return Err();
}

int64_t DebugAdapterContext::IdForFrame(Frame* frame, int stack_index) {
  FrameRecord record = {};
  record.thread_koid = frame->GetThread()->GetKoid();
  record.stack_index = stack_index;

  for (auto const& it : id_to_frame_) {
    if (it.second.thread_koid == record.thread_koid && it.second.stack_index == stack_index) {
      return it.first;
    }
  }

  int current_frame_id = next_frame_id_++;
  id_to_frame_[current_frame_id] = record;
  return current_frame_id;
}

Frame* DebugAdapterContext::FrameforId(int64_t id) {
  // id - 0 is invalid
  if (!id) {
    return nullptr;
  }

  if (auto it = id_to_frame_.find(id); it != id_to_frame_.end()) {
    Thread* thread = GetThread(it->second.thread_koid);
    if (!thread) {
      return nullptr;
    }
    if (thread->GetStack().size() <= static_cast<size_t>(it->second.stack_index)) {
      return nullptr;
    }
    return thread->GetStack()[it->second.stack_index];
  }
  // Not found
  return nullptr;
}

void DebugAdapterContext::DeleteFrameIdsForThread(Thread* thread) {
  auto thread_koid = thread->GetKoid();
  for (auto it = id_to_frame_.begin(); it != id_to_frame_.end();) {
    if (it->second.thread_koid == thread_koid) {
      DeleteVariablesIdsForFrameId(it->first);
      it = id_to_frame_.erase(it);
    } else {
      it++;
    }
  }
}

int64_t DebugAdapterContext::IdForVariables(int64_t frame_id, VariablesType type,
                                            std::unique_ptr<FormatNode> parent,
                                            fxl::WeakPtr<FormatNode> child) {
  // Check if an entry exists already, except for kChildVariable records, as those are always
  // created newly.
  if (type != VariablesType::kChildVariable) {
    for (auto const& it : id_to_variables_) {
      if (it.second.frame_id == frame_id && it.second.type == type) {
        return it.first;
      }
    }
  }

  VariablesRecord record;
  record.frame_id = frame_id;
  record.type = type;
  record.parent = std::move(parent);
  record.child = std::move(child);

  int current_variables_id = next_variables_id_++;
  id_to_variables_[current_variables_id] = std::move(record);
  return current_variables_id;
}

VariablesRecord* DebugAdapterContext::VariablesRecordForID(int64_t id) {
  // id - 0 is invalid
  if (!id) {
    return nullptr;
  }

  if (auto it = id_to_variables_.find(id); it != id_to_variables_.end()) {
    return &it->second;
  }
  // Not found
  return nullptr;
}

void DebugAdapterContext::DeleteVariablesIdsForFrameId(int64_t id) {
  for (auto it = id_to_variables_.begin(); it != id_to_variables_.end();) {
    if (it->second.frame_id == id) {
      it = id_to_variables_.erase(it);
    } else {
      it++;
    }
  }
}

}  // namespace zxdb
