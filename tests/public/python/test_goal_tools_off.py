"""Check Vibe 3.1.5's real agent tool roster under Vibe Work's child environment.

Depends on the pinned Vibe installation. Run by the vw-supervisor env test
pinned_vibe_offers_no_goal_tools_with_the_child_environment, which passes the
VIBE_DISABLED_TOOLS value the supervisor sets for every Vibe child.
"""

import asyncio
import tempfile
from pathlib import Path

from vibe.app_server._goal import GoalController
from vibe.app_server._goal_store import UnifiedGoalStore
from vibe.app_server._provided_tools import vibe_tool_groups
from vibe.app_server._unified_goal import UnifiedSessionGoal
from vibe.app_server._unified_harness_backend_adapter import UnifiedHarnessBackendAdapter
from vibe.core.config.layers.environment import EnvironmentLayer
from vibe.core.config.vibe_schema import VibeConfigSchema


GOAL_TOOLS = {"create_goal", "get_goal", "update_goal"}


class Host:
    def require_session(self, session_id):
        assert session_id == "test-session"

    def goals_available(self):
        return True

    def max_token_budget(self):
        return None

    def goal_changed(self, change):
        pass


class Hooks:
    def publish_goal_patch(self, patch):
        pass

    def publish_notice(self, message):
        pass

    async def tell_model(self, note):
        pass

    def record_goal_event(self, name, properties):
        pass

    def goal_notice_recorded(self, goal_id, event):
        return False


class ContinuationProbe:
    def __init__(self, goal):
        self._goal = type("SavedGoal", (), {"goal": goal})()

    def _user_work_waiting(self):
        raise AssertionError("a paused goal must return before continuation checks")


async def scripted_turn(available):
    """The test agent tries to create and finish a goal if offered that tool."""
    if "create_goal" not in available:
        print("scripted turn: create_goal absent")
        return
    with tempfile.TemporaryDirectory(prefix="vw-goal-off-") as temp:
        store = UnifiedGoalStore(Path(temp) / "goal.json", persistent=lambda: True)
        await store.restore()
        controller = GoalController(Host(), store)
        created = await controller.run_tool(
            "create_goal", {"objective": "Disposable tool roster test"}, call_id="create"
        )
        print(f"scripted turn: create_goal {created.goal.status.value}")
        if "update_goal" in available:
            completed = await controller.run_tool(
                "update_goal", {"status": "complete"}, call_id="complete"
            )
            print(f"scripted turn: update_goal {completed.goal.status.value}")


async def restored_goal():
    with tempfile.TemporaryDirectory(prefix="vw-goal-restore-") as temp:
        session_dir = Path(temp)
        store = UnifiedGoalStore(session_dir / "goal.json", persistent=lambda: True)
        await store.restore()
        controller = GoalController(Host(), store)
        created = await controller.run_tool(
            "create_goal", {"objective": "Disposable saved goal"}, call_id="saved"
        )
        assert created.goal.status.value == "active"
        reopened = UnifiedSessionGoal(
            session_dir,
            hooks=Hooks(),
            require_session=Host().require_session,
            available=lambda: True,
            max_token_budget=lambda: None,
            persistent=lambda: True,
        )
        await reopened.restore()
        assert reopened.goal.status.value == "paused"
        print("saved goal on reopen: paused")
        await UnifiedHarnessBackendAdapter._continue_goal_if_idle(
            ContinuationProbe(reopened.goal)
        )
        print("saved goal continuation: no turn started")


def main():
    config = EnvironmentLayer(schema=VibeConfigSchema)._read_environment()
    disabled = config.get("disabled_tools", [])
    groups = vibe_tool_groups(
        {"todo"}, enabled_tools=["*"], disabled_tools=disabled, goals=True
    )
    available = {tool.name for group in groups for tool in group.tools}
    print(f"disabled_tools={disabled}")
    print(f"goal_tools_offered={sorted(available & GOAL_TOOLS)}")
    print(f"normal_tools_offered={sorted(available & {'todo', 'cron', 'unified_harness_scratchpad'})}")
    asyncio.run(scripted_turn(available))
    asyncio.run(restored_goal())
    assert not (available & GOAL_TOOLS), "goal tools reached the scripted agent"
    assert {"todo", "cron", "unified_harness_scratchpad"} <= available


if __name__ == "__main__":
    main()
