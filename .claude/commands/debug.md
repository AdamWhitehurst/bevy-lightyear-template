# Debug

You are tasked with helping debug issues during manual testing or implementation. This command allows you to investigate problems by examining logs, database state, and git history without editing files. Think of this as a way to bootstrap a debugging session without using the primary window's context.

## Initial Response

When invoked WITH a plan/task file:
```
I'll help debug issues with [file name]. Let me understand the current state.

What specific problem are you encountering?
- What were you trying to test/implement?
- What went wrong?
- Any error messages?

I'll investigate the logs, database, and git state to help figure out what's happening.
```

When invoked WITHOUT parameters:
```
I'll help debug your current issue.

Please describe what's going wrong:
- What are you working on?
- What specific problem occurred?
- When did it last work?

I can investigate logs, database state, and recent changes to help identify the issue.
```

## Environment Information

You have access to these key locations and tools:

**Build/Runtime Logs** (from cargo commands):
- Server logs: stdout/stderr from `cargo server`
- Client logs: stdout/stderr from `cargo client`
- Build errors: output from `cargo build` or `cargo check`
- Test logs: output from `cargo test-all`

**Game State Debugging**:
- Entity/component inspection via debug prints
- Network sync issues in client-server communication
- Physics simulation state and constraints
- Input handling and action processing

**Git State**:
- Check current branch, recent commits, uncommitted changes
- Similar to how `commit` and `describe_pr` commands work

**Service Status**:
- Check if server is running: `ps aux | grep server`
- Check if client is running: `ps aux | grep client`
- Network ports in use: `netstat -tulpn | grep :7000`

## Process Steps

### Step 1: Understand the Problem

After the user describes the issue:

1. **Read any provided context** (plan or task file):
   - Understand what they're implementing/testing
   - Note which phase or step they're on
   - Identify expected vs actual behavior

2. **Quick state check**:
   - Current git branch and recent commits
   - Any uncommitted changes
   - When the issue started occurring

### Step 2: Investigate the Issue

Spawn parallel Task agents for efficient investigation:

```
Task 1 - Check Recent Build/Runtime Output:
Find and analyze recent build or runtime errors:
1. Run `cargo check` to identify compilation issues
2. Check for runtime panics or errors in recent server/client runs
3. Look for networking errors, physics warnings, or system failures
4. Check for asset loading issues or missing dependencies
5. Look for stack traces in console output
Return: Key errors/warnings from build or runtime
```

```
Task 2 - Game State Analysis:
Check the current game state and configuration:
1. Review component systems and their registration in plugins
2. Check network replication settings in protocol.rs
3. Verify physics configuration and collision layers
4. Check input mappings and action state handling
5. Look for entity spawn/despawn issues or component mismatches
Return: Game state configuration and potential issues
```

```
Task 3 - Git and File State:
Understand what changed recently:
1. Check git status and current branch
2. Look at recent commits: git log --oneline -10
3. Check uncommitted changes: git diff
4. Verify expected files exist
5. Look for any file permission issues
Return: Git state and any file issues
```

### Step 3: Present Findings

Based on the investigation, present a focused debug report:

```markdown
## Debug Report

### What's Wrong
[Clear statement of the issue based on evidence]

### Evidence Found

**From Logs** (`~/.humanlayer/logs/`):
- [Error/warning with timestamp]
- [Pattern or repeated issue]

**From Code**:
- [file:line]: issue

**From Git/Files**:
- [Recent changes that might be related]
- [File state issues]

### Root Cause
[Most likely explanation based on evidence]

### Next Steps

1. **Try This First**:
   ```bash
   [Specific command or action]
   ```

2. **If That Doesn't Work**:
   - Restart server: `cargo server`
   - Restart client: `cargo client -c 1`
   - Run with debug: `RUST_LOG=debug cargo server`

### Can't Access?
Some issues might be outside my reach:
- Browser console errors (F12 in browser)
- MCP server internal state
- System-level issues

Would you like me to investigate something specific further?
```

## Important Notes

- **Focus on manual testing scenarios** - This is for debugging during implementation
- **Always require problem description** - Can't debug without knowing what's wrong
- **Read files completely** - No limit/offset when reading context
- **Think like `commit` or `describe_pr`** - Understand git state and changes
- **Guide back to user** - Some issues (browser console, MCP internals) are outside reach
- **No file editing** - Pure investigation only

## Quick Reference

**Build/Test Commands**:
```bash
cargo check                    # Quick compilation check
cargo test-all                 # Run all tests
RUST_LOG=debug cargo server  # Server with debug logging
```

**Game State Debugging**:
```bash
cargo run -- server --help    # Check server options
cargo run -- client --help    # Check client options
netstat -tulpn | grep :7000    # Check network ports
```

**Service Check**:
```bash
ps aux | grep server          # Is server running?
ps aux | grep client          # Is client running?
```

**Git State**:
```bash
git status
git log --oneline -10
git diff
```

Remember: This command helps you investigate without burning the primary window's context. Perfect for when you hit an issue during manual testing and need to dig into logs, database, or git state.