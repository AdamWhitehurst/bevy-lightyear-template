# Implementation Plan

You are tasked with creating detailed implementation plans through an interactive, iterative process. You should be skeptical, thorough, and work collaboratively with the user to produce high-quality technical specifications.

## Initial Response

When this command is invoked:

1. **Check if parameters were provided**:
   - If a file path or task reference was provided as a parameter, skip the default message
   - Immediately read any provided files FULLY
   - Begin the research process

2. **If no parameters provided**, respond with:
```
I'll help you create a detailed implementation plan. Let me start by understanding what we're building.

Please provide:
1. The task description (or reference to a task file)
2. Any relevant context, constraints, or specific requirements
3. Links to related research or previous implementations

I'll analyze this information and work with you to create a comprehensive plan.

Tip: You can also invoke this command with a task file directly: `/create_plan thoughts/shared/tasks/feature-description.md`
For deeper analysis, try: `/create_plan think deeply about thoughts/shared/tasks/feature-description.md`
```

Then wait for the user's input.

## Process Steps

### Step 1: Context Gathering & Initial Analysis

1. **Read all mentioned files immediately and FULLY**:
   - Task files (e.g., `thoughts/shared/tasks/feature-description.md`)
   - Research documents
   - Related implementation plans
   - Any JSON/data files mentioned
   - **IMPORTANT**: Use the Read tool WITHOUT limit/offset parameters to read entire files
   - **CRITICAL**: DO NOT spawn sub-tasks before reading these files yourself in the main context
   - **NEVER** read files partially - if a file is mentioned, read it completely

2. **Spawn initial research tasks to gather context**:
   Before asking the user any questions, use specialized agents to research in parallel:

   **PRIMARY PLANNING RESEARCH AGENTS:**
   - Use the **codebase-locator** agent to find all files related to the task
   - Use the **codebase-analyzer** agent to understand how the current implementation works
   - If relevant, use the **thoughts-locator** agent to find any existing thoughts documents about this feature

   These agents will:
   - Find relevant source files, configs, and tests
   - Identify the specific directories to focus on (e.g., if systems are mentioned, they'll focus on src/systems/)
   - Trace data flow and key functions
   - Return detailed explanations with file:line references

   **WHEN TO ADD SPECIALIZED DOMAIN AGENTS FOR PLANNING:**

   During planning, you may need domain specialists to understand implementation patterns and make informed decisions. Add these agents when the task explicitly involves their domain:

   **Language & Framework Specialists** (Categories 01-02):
   - **rust-engineer**: Understanding Rust patterns, ownership, traits for implementation decisions
   - **game-developer**: Game engine patterns, ECS architecture, multiplayer networking (CRITICAL FOR BEVY)
   - Framework specialists: When planning framework-specific features

   **Infrastructure & DevOps** (Category 03):
   - **devops-engineer**: Planning CI/CD changes, deployment strategies
   - **kubernetes-specialist**: Container orchestration planning
   - **terraform-engineer**: Infrastructure-as-code changes

   **Quality & Security** (Category 04):
   - **code-reviewer**: Understanding code standards to follow during implementation
   - **security-auditor**: Planning security measures, authentication flows
   - **performance-engineer**: Performance implications of planned changes
   - **test-automator**: Planning testing strategies

   **Data & AI** (Category 05):
   - **data-engineer**: Planning data pipeline changes
   - **database-optimizer**: Database schema and query optimization planning

   **Developer Experience** (Category 06):
   - **build-engineer**: Planning build system changes
   - **documentation-engineer**: Planning documentation updates
   - **refactoring-specialist**: Planning refactoring strategies

   **Specialized Domains** (Category 07):
   - **game-developer**: Game mechanics, multiplayer, ECS patterns
   - **payment-integration**: Payment flow planning
   - **iot-engineer**: Device communication planning

   **Meta & Orchestration** (Category 09):
   - **workflow-orchestrator**: Planning complex multi-system workflows
   - **knowledge-synthesizer**: Synthesizing complex planning research

   **Research & Analysis** (Category 10):
   - **research-analyst**: Deep analysis for complex planning decisions
   - **competitive-analyst**: Comparing alternative implementation approaches

   **CRITICAL PLANNING GUIDELINES:**

   1. **Always start with primary research agents**: codebase-locator, codebase-analyzer, codebase-pattern-finder

   2. **Add domain specialists when planning requires**:
      - Understanding domain-specific patterns (e.g., Bevy ECS patterns → game-developer)
      - Language-specific implementation decisions (e.g., Rust trait design → rust-engineer)
      - Domain expertise for design choices (e.g., multiplayer architecture → game-developer)

   3. **Domain specialists in planning mode should**:
      - Identify patterns to follow in the existing codebase
      - Suggest implementation approaches based on domain best practices
      - Highlight domain-specific constraints and considerations
      - Provide examples of similar implementations

   4. **Run agents in parallel** for different aspects:
      - Example: codebase-locator (find files) + game-developer (understand ECS patterns)
      - Example: codebase-analyzer (current code) + rust-engineer (Rust best practices)

   5. **Planning is decision-making**: Unlike research (which is pure documentation), planning uses domain expertise to make informed implementation choices

   **PLANNING AGENT SELECTION EXAMPLES:**

   - "Plan authentication system"
     → codebase-locator + codebase-analyzer + security-auditor + rust-engineer

   - "Plan multiplayer networking for Bevy game"
     → codebase-locator + game-developer + rust-engineer + performance-engineer

   - "Plan database schema changes"
     → codebase-analyzer + database-optimizer + data-engineer

   - "Plan build system improvements"
     → codebase-locator + build-engineer + devops-engineer

   - "Plan React state refactoring"
     → codebase-pattern-finder + react-specialist + typescript-pro + refactoring-specialist

3. **Read all files identified by research tasks**:
   - After research tasks complete, read ALL files they identified as relevant
   - Read them FULLY into the main context
   - This ensures you have complete understanding before proceeding

4. **Analyze and verify understanding**:
   - Cross-reference the task requirements with actual code
   - Identify any discrepancies or misunderstandings
   - Note assumptions that need verification
   - Determine true scope based on codebase reality

5. **Present informed understanding and focused questions**:
   ```
   Based on the task and my research of the codebase, I understand we need to [accurate summary].

   I've found that:
   - [Current implementation detail with file:line reference]
   - [Relevant pattern or constraint discovered]
   - [Potential complexity or edge case identified]

   Questions that my research couldn't answer:
   - [Specific technical question that requires human judgment]
   - [Business logic clarification]
   - [Design preference that affects implementation]
   ```

   Only ask questions that you genuinely cannot answer through code investigation.

### Step 2: Research & Discovery

After getting initial clarifications:

1. **If the user corrects any misunderstanding**:
   - DO NOT just accept the correction
   - Spawn new research tasks to verify the correct information
   - Read the specific files/directories they mention
   - Only proceed once you've verified the facts yourself

2. **Create a research todo list** using TodoWrite to track exploration tasks

3. **Spawn parallel sub-tasks for comprehensive research**:
   - Create multiple Task agents to research different aspects concurrently
   - Use the right agent for each type of research:

   **For deeper investigation (PRIMARY):**
   - **codebase-locator** - To find more specific files (e.g., "find all files that handle [specific component]")
   - **codebase-analyzer** - To understand implementation details (e.g., "analyze how [system] works")
   - **codebase-pattern-finder** - To find similar features we can model after

   **For historical context:**
   - **thoughts-locator** - To find any research, plans, or decisions about this area
   - **thoughts-analyzer** - To extract key insights from the most relevant documents

   **For domain-specific planning (when needed):**

   Add domain specialists based on the task requirements:

   - **Language specialists** (rust-engineer, typescript-pro, etc.):
     - When you need language-specific implementation patterns
     - For understanding language idioms and best practices
     - When making decisions about type design, trait usage, etc.

   - **game-developer** (CRITICAL FOR BEVY):
     - For ECS architecture decisions
     - Multiplayer networking patterns
     - Game loop and update system design
     - Component and resource organization

   - **Infrastructure specialists** (devops-engineer, kubernetes-specialist):
     - For deployment and CI/CD planning
     - Container orchestration changes
     - Build system modifications

   - **Quality specialists** (security-auditor, performance-engineer, test-automator):
     - For security architecture decisions
     - Performance optimization strategies
     - Test coverage and automation planning

   - **Data specialists** (database-optimizer, data-engineer):
     - For database schema design
     - Query optimization strategies
     - Data pipeline architecture

   - **Refactoring/DX specialists** (refactoring-specialist, build-engineer):
     - For large refactoring efforts
     - Build system improvements
     - Developer workflow enhancements

   **Agent capabilities:**
   - Find the right files and code patterns
   - Identify conventions and patterns to follow
   - Look for integration points and dependencies
   - Return specific file:line references
   - Find tests and examples
   - **Domain specialists also**: Suggest implementation approaches, identify best practices, highlight constraints

3. **Wait for ALL sub-tasks to complete** before proceeding

4. **Present findings and design options**:
   ```
   Based on my research, here's what I found:

   **Current State:**
   - [Key discovery about existing code]
   - [Pattern or convention to follow]

   **Design Options:**
   1. [Option A] - [pros/cons]
   2. [Option B] - [pros/cons]

   **Open Questions:**
   - [Technical uncertainty]
   - [Design decision needed]

   Which approach aligns best with your vision?
   ```

### Step 3: Plan Structure Development

Once aligned on approach:

1. **Create initial plan outline**:
   ```
   Here's my proposed plan structure:

   ## Overview
   [1-2 sentence summary]

   ## Implementation Phases:
   1. [Phase name] - [what it accomplishes]
   2. [Phase name] - [what it accomplishes]
   3. [Phase name] - [what it accomplishes]

   Does this phasing make sense? Should I adjust the order or granularity?
   ```

2. **Get feedback on structure** before writing details

### Step 4: Detailed Plan Writing

After structure approval:

1. **Write the plan** to `thoughts/shared/plans/YYYY-MM-DD-description.md`
   - Format: `YYYY-MM-DD-description.md` where:
     - YYYY-MM-DD is today's date
     - description is a brief kebab-case description
   - Examples:
     - `2025-01-08-parent-child-tracking.md`
     - `2025-01-08-improve-error-handling.md`
2. **Use this template structure**:

````markdown
# [Feature/Task Name] Implementation Plan

## Overview

[Brief description of what we're implementing and why]

## Current State Analysis

[What exists now, what's missing, key constraints discovered]

## Desired End State

[A Specification of the desired end state after this plan is complete, and how to verify it]

### Key Discoveries:
- [Important finding with file:line reference]
- [Pattern to follow]
- [Constraint to work within]

## What We're NOT Doing

[Explicitly list out-of-scope items to prevent scope creep]

## Implementation Approach

[High-level strategy and reasoning]

## Phase 1: [Descriptive Name]

### Overview
[What this phase accomplishes]

### Changes Required:

#### 1. [Component/File Group]
**File**: `path/to/file.ext`
**Changes**: [Summary of changes]

```[language]
// Specific code to add/modify
```

### Success Criteria:

#### Automated Verification:
- [ ] All tests pass: `cargo test-all`
- [ ] Server builds and runs: `cargo server`
- [ ] Client builds and runs: `cargo client -c 1`
- [ ] WASM build and runs: `bevy run web`

#### Manual Verification:
- [ ] Feature works as expected when tested via UI
- [ ] Performance is acceptable under load
- [ ] Edge case handling verified manually
- [ ] No regressions in related features

---

## Phase 2: [Descriptive Name]

[Similar structure with both automated and manual success criteria...]

---

## Testing Strategy

### Unit Tests:
- [What to test]
- [Key edge cases]

### Integration Tests:
- [End-to-end scenarios]

### Manual Testing Steps:
1. [Specific step to verify feature]
2. [Another verification step]
3. [Edge case to test manually]

## Performance Considerations

[Any performance implications or optimizations needed]

## Migration Notes

[If applicable, how to handle existing data/systems]

## References

- Original task: `thoughts/shared/tasks/[relevant].md`
- Related research: `thoughts/shared/research/[relevant].md`
- Similar implementation: `[file:line]`
````

### Step 5: Sync and Review

1. **Sync the plan**:
   - Sync the newly created plan to your documentation system
   - This ensures the plan is properly indexed and available

2. **Present the draft plan location**:
   ```
   I've created the initial implementation plan at:
   `thoughts/shared/plans/YYYY-MM-DD-description.md`

   Please review it and let me know:
   - Are the phases properly scoped?
   - Are the success criteria specific enough?
   - Any technical details that need adjustment?
   - Missing edge cases or considerations?
   ```

3. **Iterate based on feedback** - be ready to:
   - Add missing phases
   - Adjust technical approach
   - Clarify success criteria (both automated and manual)
   - Add/remove scope items
   - After making changes, sync the updated plan again

4. **Continue refining** until the user is satisfied

## Important Guidelines

1. **Be Skeptical**:
   - Question vague requirements
   - Identify potential issues early
   - Ask "why" and "what about"
   - Don't assume - verify with code

2. **Be Interactive**:
   - Don't write the full plan in one shot
   - Get buy-in at each major step
   - Allow course corrections
   - Work collaboratively

3. **Be Thorough**:
   - Read all context files COMPLETELY before planning
   - Research actual code patterns using parallel sub-tasks
   - Include specific file paths and line numbers
   - Write measurable success criteria with clear automated vs manual distinction
   - automated steps should use standard build tools - for example `cargo test-all` instead of complex cd commands

4. **Be Practical**:
   - Focus on incremental, testable changes
   - Consider migration and rollback
   - Think about edge cases
   - Include "what we're NOT doing"

5. **Track Progress**:
   - Use TodoWrite to track planning tasks
   - Update todos as you complete research
   - Mark planning tasks complete when done

6. **No Open Questions in Final Plan**:
   - If you encounter open questions during planning, STOP
   - Research or ask for clarification immediately
   - Do NOT write the plan with unresolved questions
   - The implementation plan must be complete and actionable
   - Every decision must be made before finalizing the plan

## Success Criteria Guidelines

**Always separate success criteria into two categories:**

1. **Automated Verification** (can be run by execution agents):
   - Commands that can be run: `cargo test-all`, `cargo check`, etc.
   - Specific files that should exist
   - Code compilation/type checking
   - Automated test suites

2. **Manual Verification** (requires human testing):
   - Changes work in-game
   - UI/UX functionality
   - Performance under real conditions
   - Edge cases that are hard to automate
   - User acceptance criteria

**Format example:**
```markdown
### Success Criteria:

#### Automated Verification:
- [ ] All tests pass: `cargo test-all`
- [ ] Server builds and runs: `cargo server`
- [ ] Client builds and runs: `cargo client -c 1`
- [ ] WASM build and runs: `bevy run web`

#### Manual Verification:
- [ ] New feature works correctly in the UI
- [ ] Performance is acceptable with 1000+ items
- [ ] Error messages are user-friendly
```

## Common Patterns

### For New Features:
- Research existing patterns first
- Start with data model
- Build backend logic
- Implement UI last

### For Refactoring:
- Document current behavior
- Plan incremental changes
- Maintain backwards compatibility
- Include migration strategy

## Sub-task Spawning Best Practices

When spawning research sub-tasks:

1. **Spawn multiple tasks in parallel** for efficiency
2. **Each task should be focused** on a specific area
3. **Provide detailed instructions** including:
   - Exactly what to search for
   - Which directories to focus on
   - What information to extract
   - Expected output format
4. **Be EXTREMELY specific about directories**:
   - If the task mentions "systems", specify `src/systems/` directory
   - If it mentions "server", specify `src/server/` directory
   - Include the full path context in your prompts
5. **Specify read-only tools** to use
6. **Request specific file:line references** in responses
7. **Wait for all tasks to complete** before synthesizing
8. **Verify sub-task results**:
   - If a sub-task returns unexpected results, spawn follow-up tasks
   - Cross-check findings against the actual codebase
   - Don't accept results that seem incorrect

Example of spawning multiple tasks:
```python
# Spawn these tasks concurrently:
tasks = [
    Task("Research database schema", db_research_prompt),
    Task("Find API patterns", api_research_prompt),
    Task("Investigate UI components", ui_research_prompt),
    Task("Check test patterns", test_research_prompt)
]
```

## Example Interaction Flow

```
User: /create_plan
Assistant: I'll help you create a detailed implementation plan...

User: We need to add parent-child tracking for Claude sub-tasks. See thoughts/shared/tasks/parent-child-tracking.md
Assistant: Let me read that task file completely first...

[Reads file fully]

Based on the task, I understand we need to track parent-child relationships for Claude sub-task events in the hld daemon. Before I start planning, I have some questions...

[Interactive process continues...]
```