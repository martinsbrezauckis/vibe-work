/** Exact native navigation, disclosure, editor and toolbar controls. Keeps existing semantics; the style guard rejects new families and unlisted classes. */
export const rawClasses = new Map([
  ["changes/HunkLines.tsx", new Set(["lc-add"])],
  ["composer/ComposerParts.tsx", new Set(["ibtn", "sm"])],
  ["composer/ComposerTools.tsx", new Set(["ibtn", "send"])],
  ["composer/DocumentChips.tsx", new Set(["ibtn", "sm"])],
  ["connectors/ConnectorTile.tsx", new Set(["ctile-main"])],
  ["files/FileRef.tsx", new Set(["fref", "md-image"])],
  ["files/FileRefLink.tsx", new Set(["fref"])],
  ["libraries/AskPanel.tsx", new Set(["lib-source"])],
  ["libraries/LibrariesPage.tsx", new Set(["lib-item"])],
  ["marketplace/MarketCard.tsx", new Set(["ctile-main"])],
  ["marketplace/MarketDetail.tsx", new Set(["linkish"])],
  ["pages/AnswersPanel.tsx", new Set(["ibtn"])],
  ["pages/Hub.tsx", new Set(["pages-plan-link"])],
  ["pages/HubRows.tsx", new Set(["pages-hub-row"])],
  ["pages/PagesToolbar.tsx", new Set(["ibtn"])],
  ["preview/CaptureMenu.tsx", new Set(["ibtn", "preview-capture-item", "preview-capture-reveal"])],
  ["preview/PageTabs.tsx", new Set(["browser-page-handoff", "browser-page-new"])],
  ["projects/ProjectCardView.tsx", new Set(["pj-name"])],
  ["projects/ProjectDetail.tsx", new Set(["pages-plan-link", "pj-x"])],
  ["projects/useProjectActions.tsx", new Set(["pj-actions"])],
  ["sessions/ProjectGroupView.tsx", new Set(["proj-h", "srow-more"])],
  ["sessions/SessionRow.tsx", new Set(["srow-main", "srow-more", "twist"])],
  ["sessions/UnfiledSection.tsx", new Set(["proj-h"])],
  ["settings/ImageSettings.tsx", new Set(["ext-link"])],
  // The level cards of Your Chrome: the vault setup's choice card as a radio.
  ["settings/chrome/YourChromeSettings.tsx", new Set(["vault-choice"])],
  ["setup/VaultStartStep.tsx", new Set(["vault-choice"])],
  ["terminal/TerminalPanel.tsx", new Set(["ibtn", "vw-term-new", "vw-term-tool"])],
  ["transcript/FindBar.tsx", new Set(["find-scope"])],
  ["transcript/TodoStrip.tsx", new Set(["todo-h"])],
  ["transcript/Transcript.tsx", new Set(["jump-latest"])],
  ["transcript/rows/MessageRows.tsx", new Set(["copy-btn"])],
  ["transcript/rows/SourcesList.tsx", new Set(["source"])],
  ["transcript/rows/StepItem.tsx", new Set(["step-h"])],
  ["transcript/rows/StepsGroup.tsx", new Set(["steps-h", "thought"])],
  ["voice/NarrationBar.tsx", new Set(["ibtn"])],
  ["worktrees/CopiesButton.tsx", new Set(["rbtn-wide"])],
]);
export const rawExpressions = new Map([
  ["connectors/ServerList.tsx", new Set(['{`srow ${selected === id ? "sel" : ""}`}'])],
  ["voice/MicButton.tsx", new Set(['{`ibtn mic mic-${mine ? phase : "idle"}`}'])],
  ["voice/VoiceChatButton.tsx", new Set(["{`vchat vchat-${phase}`}"])],
]);
export const bareControls = new Map([
  ["changes/GitReview.tsx", new Set(["changes.git-review-select"])],
  [
    "composer/ComposerParts.tsx",
    new Set([
      "composer.composer-parts-button",
      "composer.composer-parts-on-remove-context",
      "composer.composer-parts-on-remove-image",
    ]),
  ],
  ["composer/PasteChips.tsx", new Set(["composer.paste-chip-remove"])],
  ["editor/MergeCompare.tsx", new Set(["files.merge-compare-close"])],
  [
    "files/FileFindBar.tsx",
    new Set([
      "files.file-find-bar-button",
      "files.file-find-bar-step",
      "files.file-find-bar-step-2",
    ]),
  ],
  ["pages/PlanChecklist.tsx", new Set(["pages.plan-checklist-open-step"])],
  ["preview/SaveVault.tsx", new Set(["preview.save-vault-choose-3"])],
  ["projects/ProjectsPage.tsx", new Set(["projects.projects-page-open-session"])],
  [
    "transcript/FindBar.tsx",
    new Set([
      "transcript.find-bar-button",
      "transcript.find-bar-step",
      "transcript.find-bar-step-2",
    ]),
  ],
]);
export const passiveClasses = new Map([["worktrees/LocationChip.tsx", new Set(["wt-chip-text"])]]);

// Existing repository/project pickers retain their native popup and geometry in this consolidation.
// New selects, even in these files, still require Selector.
export const nativeSelectors = new Map([
  ["git/BranchBar.tsx", new Set(["git.branch-bar-select-repo"])],
  ["git-browser/register.tsx", new Set(["git-browser.project-select"])],
]);
