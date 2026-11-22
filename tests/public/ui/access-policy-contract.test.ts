/** Approval policies are separate from agent identity in Vibe 3.1.4 runtime and settings responses. */
import { describe, expect, it } from "vitest";
import {
  configuredModeId,
  currentModeId,
  offeredModes,
} from "../../../ui/src/features/composer/model/access";
import { parseVibeSettings } from "../../../ui/src/state/vibeSettings";

describe("Vibe approval policy contract", () => {
  it("shows the active policy and offers only advertised optional policies", () => {
    const runtime = {
      activeAgent: { name: "default" },
      agents: [{ name: "default" }, { name: "vw-plan" }],
      activeApprovalPolicy: { name: "ask", safety: "safe" },
      approvalPolicies: [
        { name: "ask", safety: "safe" },
        { name: "smart-approve", safety: "safe" },
      ],
    };
    expect(currentModeId(runtime, { agent: { name: "default" } })).toBe("ask");
    expect(offeredModes(runtime).map((mode) => mode.id)).toContain(
      "smart-approve",
    );
    expect(offeredModes(runtime).map((mode) => mode.id)).not.toContain(
      "vw-plan",
    );
    expect(offeredModes(runtime).map((mode) => mode.id)).not.toContain(
      "auto-approve",
    );
  });

  it("uses the approval default and fails closed on missing policy", () => {
    expect(
      configuredModeId({
        config: {
          defaultAgent: "default",
          defaultApprovalPolicy: "accept-edits",
        },
      }),
    ).toBe("accept-edits");
    expect(
      currentModeId({ activeAgent: { name: "default" } }, { agent: "default" }),
    ).toBeNull();
    expect(configuredModeId({ config: { defaultAgent: "lean" } })).toBeNull();
  });

  it("reads the approval default setting independently of default_agent", () => {
    const settings = parseVibeSettings({
      fields: [
        { name: "default_agent", value: "default", layerValues: [] },
        {
          name: "default_approval_policy",
          value: "auto-approve",
          layerValues: [],
        },
      ],
    });
    expect(settings?.defaultApprovalPolicy.value).toBe("auto-approve");
  });

  it("maps a saved Plan mode to Ask and never invents smart approve", () => {
    const runtime = {
      activeAgent: { name: "vw-plan" },
      activeApprovalPolicy: { name: "ask" },
      agents: [{ name: "default" }, { name: "vw-plan" }],
      approvalPolicies: [{ name: "ask" }],
    };
    expect(currentModeId(runtime, undefined, "vw-plan")).toBe("ask");
    expect(
      currentModeId(
        { activeApprovalPolicy: { name: "auto-approve" } },
        undefined,
        "vw-plan",
      ),
    ).toBe("auto-approve");
    expect(offeredModes(runtime).map((mode) => mode.id)).not.toContain(
      "smart-approve",
    );
    expect(
      currentModeId({ activeAgent: { name: "lean" } }, undefined),
    ).toBeNull();
    expect(
      currentModeId({ activeApprovalPolicy: { name: "ask" } }, undefined),
    ).toBe("ask");
    expect(
      currentModeId(
        { activeApprovalPolicy: { name: "auto-approve" } },
        undefined,
      ),
    ).toBe("auto-approve");
    expect(
      currentModeId(
        { activeApprovalPolicy: { name: "accept-edits" } },
        undefined,
      ),
    ).toBe("accept-edits");
    expect(currentModeId({}, undefined)).toBeNull();
    expect(currentModeId({}, { agent: { name: "vw-plan" } })).toBe("ask");
    expect(
      configuredModeId({
        config: {
          defaultAgent: "vw-plan",
          defaultApprovalPolicy: "auto-approve",
        },
      }),
    ).toBe("auto-approve");
  });
});
