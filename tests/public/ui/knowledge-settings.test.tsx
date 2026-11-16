/**
 * Settings > Knowledge base through the real shell and a typed fake core; synthetic data, no network.
 *
 * PREDICT (written before the page existed): opening the "knowledge" category falls back to About, so there is no
 * "Knowledge base" region and every test below fails at its first query; Settings search finds nothing for "our
 * library". With the page: two groups, Add for a folder (native picker) and a cloud drive, Add for a Mistral Library
 * from the listed libraries, Rename, Remove, and the core's refusal shown inline.
 */
import {
  act,
  fireEvent,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import type {
  Command,
  CommandResult,
  KnowledgePlace,
  LibraryView,
} from "../../../ui/src/protocol/core";
import { searchSettings } from "../../../ui/src/features/settings/catalog";
import { renderShell } from "./support/renderShell";

type KnowledgeCommand = Extract<Command, { type: "knowledge" }>;

const LIBRARY: LibraryView = {
  id: "lib-1",
  name: "Research papers",
  description: null,
} as unknown as LibraryView;

async function setup(initial: KnowledgePlace[] = []) {
  let places = initial;
  let next = 0;
  const sent: KnowledgeCommand["request"][] = [];
  const r = await renderShell({
    setup(client) {
      client.answerWith("knowledge", (command): CommandResult => {
        const request = (command as KnowledgeCommand).request;
        sent.push(request);
        if (request.op === "add") {
          if (request.name === "ghp_fake")
            throw new Error("The name looks like a password or key.");
          next += 1;
          places = [
            ...places,
            {
              id: `kb-${String(next)}`,
              name: request.name,
              kind: request.kind,
              location: request.location,
              ...(request.label === undefined ? {} : { label: request.label }),
              ...(request.note === undefined ? {} : { note: request.note }),
            },
          ];
        } else if (request.op === "rename") {
          places = places.map((p) =>
            p.id === request.id ? { ...p, name: request.name } : p,
          );
        } else if (request.op === "remove") {
          places = places.filter((p) => p.id !== request.id);
        }
        return { type: "knowledge", view: { places } };
      });
      client.answer("libraries", {
        type: "libraries",
        result: {
          kind: "list",
          libraries: [LIBRARY],
          partial: null,
          costs: {},
        },
      } as unknown as CommandResult);
    },
  });
  act(() => {
    r.app.shell.actions.openSettings("knowledge");
  });
  const page = await screen.findByRole("region", { name: "Knowledge base" });
  return { r, page, sent };
}

describe("Knowledge base page", () => {
  it("shows two groups and says it is a pointer only", async () => {
    const { page } = await setup();
    expect(
      within(page).getByRole("heading", {
        level: 4,
        name: "In your Mistral account",
      }),
    ).toBeInTheDocument();
    expect(
      within(page).getByRole("heading", {
        level: 4,
        name: "On this computer and your drives",
      }),
    ).toBeInTheDocument();
    expect(page).toHaveTextContent(
      "it gives the agent no extra access, and nothing is copied",
    );
    expect(
      await within(page).findByText("No folders or drives added."),
    ).toBeInTheDocument();
  });

  it("adds a folder from the native picker and an online-only drive", async () => {
    const { r, page, sent } = await setup();
    r.client.pickFolder.mockResolvedValueOnce("/home/me/Library");
    fireEvent.click(
      await within(page).findByRole("button", {
        name: "Add a folder or drive",
      }),
    );
    fireEvent.change(within(page).getByLabelText("Name you use for it"), {
      target: { value: "Our library" },
    });
    fireEvent.click(
      within(page).getByRole("button", { name: "Choose folder…" }),
    );
    await waitFor(() =>
      expect(within(page).getByLabelText("Folder")).toHaveValue(
        "/home/me/Library",
      ),
    );
    fireEvent.click(within(page).getByRole("button", { name: "Add" }));
    expect(await within(page).findByText("Our library")).toBeInTheDocument();
    expect(within(page).getByText("/home/me/Library")).toBeInTheDocument();
    expect(sent).toContainEqual({
      op: "add",
      name: "Our library",
      kind: "folder",
      location: "/home/me/Library",
    });

    fireEvent.click(
      within(page).getByRole("button", { name: "Add a folder or drive" }),
    );
    const user = userEvent.setup();
    await user.click(within(page).getByRole("combobox", { name: "Kind" }));
    await user.click(
      await screen.findByRole("option", {
        name: /Cloud drive not on this computer/,
      }),
    );
    fireEvent.change(within(page).getByLabelText("Name you use for it"), {
      target: { value: "Team drive" },
    });
    fireEvent.change(within(page).getByLabelText("Connector"), {
      target: { value: "Google Drive" },
    });
    fireEvent.change(
      within(page).getByLabelText("Note for the agent (optional)"),
      {
        target: { value: "offers" },
      },
    );
    fireEvent.click(within(page).getByRole("button", { name: "Add" }));
    expect(
      await within(page).findByText(
        "Cloud drive, through your Google Drive connector",
      ),
    ).toBeInTheDocument();
    expect(sent).toContainEqual({
      op: "add",
      name: "Team drive",
      kind: "drive",
      location: "Google Drive",
      note: "offers",
    });
  });

  it("adds a Mistral Library chosen from the listed libraries", async () => {
    const { r, page, sent } = await setup();
    fireEvent.click(
      await within(page).findByRole("button", { name: "Add a library" }),
    );
    await waitFor(() => {
      expect(r.client.commands).toContainEqual({
        type: "libraries",
        request: { op: "list" },
      });
    });
    const user = userEvent.setup();
    await user.click(
      await within(page).findByRole("combobox", { name: "Library" }),
    );
    await user.click(
      await screen.findByRole("option", { name: /Research papers/ }),
    );
    expect(within(page).getByLabelText("Name you use for it")).toHaveValue(
      "Research papers",
    );
    fireEvent.click(within(page).getByRole("button", { name: "Add" }));
    expect(
      await within(page).findByText("Mistral Library Research papers"),
    ).toBeInTheDocument();
    expect(sent).toContainEqual({
      op: "add",
      name: "Research papers",
      kind: "mistralLibrary",
      location: "lib-1",
      label: "Research papers",
    });
  });

  it("renames and removes a place, and shows a refusal inline", async () => {
    const { page, sent } = await setup([
      { id: "kb-a", name: "Library", kind: "folder", location: "/srv/library" },
    ]);
    fireEvent.click(
      await within(page).findByRole("button", { name: "Rename Library" }),
    );
    fireEvent.change(within(page).getByLabelText("New name for Library"), {
      target: { value: "Our library" },
    });
    fireEvent.click(within(page).getByRole("button", { name: "Save" }));
    expect(await within(page).findByText("Our library")).toBeInTheDocument();
    expect(sent).toContainEqual({
      op: "rename",
      id: "kb-a",
      name: "Our library",
    });

    fireEvent.click(
      within(page).getByRole("button", { name: "Add a folder or drive" }),
    );
    fireEvent.change(within(page).getByLabelText("Name you use for it"), {
      target: { value: "ghp_fake" },
    });
    fireEvent.click(within(page).getByRole("button", { name: "Add" }));
    // Shown inline; said once by the app's announcer (the paragraph is no alert of its own).
    expect(
      await within(page).findByText(/looks like a password or key/),
    ).toBeInTheDocument();
    fireEvent.click(within(page).getByRole("button", { name: "Cancel" }));

    fireEvent.click(
      within(page).getByRole("button", { name: "Remove Our library" }),
    );
    expect(
      await within(page).findByText("No folders or drives added."),
    ).toBeInTheDocument();
    expect(sent).toContainEqual({ op: "remove", id: "kb-a" });
  });

  it("is found by Settings search", () => {
    for (const words of ["our library", "knowledge", "onedrive"]) {
      expect(
        searchSettings(words).some((hit) => hit.category === "knowledge"),
        words,
      ).toBe(true);
    }
  });
});
