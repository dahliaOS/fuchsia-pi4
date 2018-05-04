// Copyright 2016 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#include "peridot/bin/suggestion_engine/ranked_suggestions_list.h"

#include <algorithm>
#include <string>

#include "lib/context/cpp/context_helper.h"
#include "lib/fxl/logging.h"

namespace modular {

MatchPredicate GetSuggestionMatcher(const std::string& component_url,
                                    const std::string& proposal_id) {
  return [component_url,
          proposal_id](const std::unique_ptr<RankedSuggestion>& suggestion) {
    return (suggestion->prototype->proposal.id == proposal_id) &&
           (suggestion->prototype->source_url == component_url);
  };
}

MatchPredicate GetSuggestionMatcher(const std::string& suggestion_id) {
  return [suggestion_id](const std::unique_ptr<RankedSuggestion>& suggestion) {
    return suggestion->prototype->suggestion_id == suggestion_id;
  };
}

RankedSuggestionsList::RankedSuggestionsList() {}

RankedSuggestionsList::~RankedSuggestionsList() = default;

void RankedSuggestionsList::SetRanker(std::unique_ptr<Ranker> ranker) {
  ranker_ = std::move(ranker);
}

RankedSuggestion* RankedSuggestionsList::GetMatchingSuggestion(
    MatchPredicate matchFunction) const {
  auto findIter =
      std::find_if(suggestions_.begin(), suggestions_.end(), matchFunction);
  if (findIter != suggestions_.end())
    return findIter->get();
  return nullptr;
}

bool RankedSuggestionsList::RemoveMatchingSuggestion(
    MatchPredicate matchFunction) {
  auto remove_iter =
      std::remove_if(suggestions_.begin(), suggestions_.end(), matchFunction);
  if (remove_iter == suggestions_.end()) {
    return false;
  } else {
    suggestions_.erase(remove_iter, suggestions_.end());
    return true;
  }
}

void RankedSuggestionsList::Rank(const UserInput& query) {
  if (!ranker_) {
    FXL_LOG(WARNING)
        << "RankedSuggestionList.Rank ignored since no ranker was set.";
    return;
  }
  for (auto& suggestion : suggestions_) {
    suggestion->confidence = ranker_->Rank(query, *suggestion);
    FXL_VLOG(1) << "Proposal "
                << suggestion->prototype->proposal.display.headline
                << " confidence " << suggestion->prototype->proposal.confidence
                << " => " << suggestion->confidence;
  }
  DoStableSort();
}

void RankedSuggestionsList::AddSuggestion(SuggestionPrototype* prototype) {
  std::unique_ptr<RankedSuggestion> ranked_suggestion =
      std::make_unique<RankedSuggestion>();
  ranked_suggestion->prototype = prototype;
  suggestions_.push_back(std::move(ranked_suggestion));
}

bool RankedSuggestionsList::RemoveProposal(const std::string& component_url,
                                           const std::string& proposal_id) {
  return RemoveMatchingSuggestion(
      GetSuggestionMatcher(component_url, proposal_id));
}

RankedSuggestion* RankedSuggestionsList::GetSuggestion(
    const std::string& suggestion_id) const {
  return GetMatchingSuggestion(GetSuggestionMatcher(suggestion_id));
}

RankedSuggestion* RankedSuggestionsList::GetSuggestion(
    const std::string& component_url,
    const std::string& proposal_id) const {
  return GetMatchingSuggestion(
      GetSuggestionMatcher(component_url, proposal_id));
}

void RankedSuggestionsList::RemoveAllSuggestions() {
  suggestions_.clear();
}

// Start of private sorting methods.

void RankedSuggestionsList::DoStableSort() {
  std::stable_sort(suggestions_.begin(), suggestions_.end(),
                   [](const std::unique_ptr<RankedSuggestion>& a,
                      const std::unique_ptr<RankedSuggestion>& b) {
                     return a->confidence > b->confidence;
                   });
}

// End of private sorting methods.

}  // namespace modular
