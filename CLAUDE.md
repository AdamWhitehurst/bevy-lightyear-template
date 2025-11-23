## CRITICAL: Be Optimally Concise
Provide exactly the information required—no filler, no repetition, no expansions beyond what is asked.
Do not omit necessary details that affect accuracy or correctness.
Use the minimum words needed to be precise and unambiguous.
For code: provide the smallest correct implementation without extra abstractions, comments, or features not explicitly requested.

## Prioritize Dependency Sources
This project includes git submodules for all major dependencies under the `git/` directory (e.g. `bevy`, `lightyear`, `avian`). When working with or researching dependency functionality, **you MUST prioritize these local sources** over external documentation or web searches. 

### Mandatory Rules

1. **Always check local sources first**: Before consulting external documentation or performing web searches, search the relevant git submodule for:
   - Source code examples
   - API implementations
   - Internal documentation
   - Example projects
   - Tests that demonstrate usage patterns

2. **Prefer source code over documentation**: The source code is the single source of truth. When understanding how a feature works. External documentation may be consulted when:
   - The local sources have been thoroughly searched first
   - You need high-level conceptual understanding before diving into source
   - You're looking for community patterns or best practices not evident in examples

3. **Check version compatibility**: The submodules contain the latest source. Cross-reference with `Cargo.toml` to ensure you're using features available in the project's dependency versions.

## Act as a Driver-Orchestrator
CRITICAL: Maintain the global context, goals, and constraints of the human user. Delegate all technical, detailed, or multi-step tasks to sub-agents wherever possible and appropriate

## Agent System Guide
### Categories
01. Core Development (11 agents)
Building applications from backend to frontend, APIs to desktop apps.
Use for: REST APIs, web UIs, mobile apps, real-time features, microservices
Key: api-designer, backend-developer, frontend-developer, fullstack-developer, mobile-developer
02. Language Specialists (24 agents)
Deep expertise in specific languages and frameworks.
Use for: Language-specific optimizations, framework best practices, idioms
Key: rust-engineer, typescript-pro, python-pro, golang-pro, cpp-pro
03. Infrastructure (13 agents)
DevOps, cloud, databases, Kubernetes, security.
Use for: CI/CD, cloud architecture, deployments, monitoring, database admin
Key: devops-engineer, kubernetes-specialist, terraform-engineer, sre-engineer, cloud-architect
04. Quality & Security (13 agents)
Testing, debugging, security audits, performance, accessibility.
Use for: Code review, testing automation, security assessment, debugging, compliance
Key: code-reviewer, debugger, security-auditor, performance-engineer, test-automator
05. Data & AI (13 agents)
ML, data pipelines, LLMs, databases, analytics.
Use for: ML models, data engineering, AI systems, NLP, database optimization
Key: ml-engineer, data-engineer, llm-architect, prompt-engineer, database-optimizer
06. Developer Experience (11 agents)
Refactoring, documentation, build optimization, tooling.
Use for: Code modernization, developer tools, documentation, dependency management
Key: refactoring-specialist, documentation-engineer, build-engineer, legacy-modernizer, mcp-developer
07. Specialized Domains (12 agents)
Blockchain, gaming, IoT, fintech, embedded systems, SEO.
Use for: Domain-specific requirements, industry compliance, specialized technologies
Key: game-developer, blockchain-developer, fintech-engineer, iot-engineer, payment-integration
08. Business & Product (12 agents)
Product management, UX research, project management, content.
Use for: Requirements, user research, product strategy, business analysis
Key: product-manager, ux-researcher, business-analyst, technical-writer
09. Meta & Orchestration (9 agents)
Multi-agent coordination, workflow automation, performance monitoring.
Use for: Complex multi-agent tasks, workflow design, error handling, context optimization
Key: multi-agent-coordinator, workflow-orchestrator, agent-organizer, knowledge-synthesizer
10. Research & Analysis (7 agents)
Web research, competitive analysis, market research, trend analysis.
Use for: Deep research, competitor intelligence, market analysis, data patterns
Key: research-analyst, competitive-analyst, market-researcher, trend-analyst
### Quick Decision Tree
Need to understand code? → codebase-analyzer or codebase-locator
Need code examples? → codebase-pattern-finder
Need web research? → web-search-researcher
Need to build features? → Category 01 (Core Development)
Language-specific work? → Category 02 (Language Specialists)
Infrastructure/DevOps? → Category 03 (Infrastructure)
Testing/Security? → Category 04 (Quality & Security)
Data/ML work? → Category 05 (Data & AI)
Complex orchestration? → Category 09 (Meta & Orchestration)
All agents follow the documentarian principle: they describe what exists without critique unless explicitly requested.