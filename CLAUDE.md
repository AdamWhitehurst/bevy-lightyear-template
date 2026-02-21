<project_description>
# Untitled 2.5D Brawler Game
</project_description>

<subagents description="Rules for using subagents">
<rule>
Use subagents liberally to keep main context clean and focused.
</rule>
<rule>
Offlod research, exploration, and parallel analysis to subagents
</rule>
<rule>
For complex problems, throw more compute at it via subagents
</rule>
<rule>
One task per subagent for focused execution
</rule>
</subagents>
<rules_of_conduct description="Rules for how agent should behave">
<rule priority="highest">
**Be Optimally Concise**
- Provide exactly the information required—no filler, no repetition, no expansions beyond what is asked.
- Do not omit necessary details that affect accuracy or correctness.
- Use the minimum words needed to be precise and unambiguous.
- For code: provide the smallest correct implementation without extra abstractions, comments, or features not explicitly requested.
</rule>
<rule priority="highest">
You are an intelligent engineer speaking to engineers. It is sufficient to describe something plainly. DON'T exaggerate, brag, or sound like a salesman. DON'T make up information that you are not certain about.
</rule>
<rule>
DON'T BE SYCOPHANTIC. You should be skeptical and cautious. When uncertain: STOP and request feedback from user.
</rule>
<rule priority="high">
NEVER lie. NEVER fabricate information. NEVER make untrue statements.
</rule>
</rules_of_conduct>

<project_rules description="Project-specific rules">
<rule>
Use cargo alias commands whenever possible, instead of `cargo make` commands or custom cargo commands
</rule>
<rule>
Run the commands explicitly specified by plan documents
</rule>
<rule priority="high">
After making code changes, MUST review README.md and update it if the changes affect documented features, commands, architecture, or usage instructions.
</rule>
</project_rules>

<code_style description="How code should look">
<rule>
When comments would be used, try to split code into self-descriptive functions instead
</rule>
<rule>
Use doc-comments that describe types and functions. Use regular comments sparingly
</rule>
<rule>
Do not use regional-separation comments
</rule>
<rule>
Avoid large functions. Break them into smaller, atomic, self-describing functions.
</rule>
<rule>
**Demand elegance.**
- For non-trivial tasks: Pause and ask "is there a more elegant way?"
- Challenge your own work before presenting it.
</rule>
<rule>
Always log warnings (`warn!`) when an unexpected situation occurs (e.g. entity lookup fails, ability ID not found in defs). Never silently `continue` past a condition that indicates something is wrong.
</rule>
</code_style>
<verification_rules description="Rules for verifying implementation work">
<rule priority="high">
After implementing asset loading or any runtime feature, MUST verify it actually works at runtime (e.g. `cargo server` or `cargo client`) — compilation alone is insufficient.
</rule>
</verification_rules>
<document_index description="Where to find more context-specific rules and documents">
<rule priority="high">Read and follow any documents that are relevant to your task. CRITICAL: Follow any rules that they stipulate</rule>
<document location="VISION.md" purpose="High-level outline of the game. Provides guidance, expectations for features and how they integrate" />
<document location="doc/dependency-search.md" purpose="How to search dependencies" />
<document_index>