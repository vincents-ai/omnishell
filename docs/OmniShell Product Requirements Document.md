# **Product Requirements Document (PRD): OmniShell**

## **1\. Executive Summary**

**OmniShell** is a natively compiled, POSIX-compliant interactive terminal environment built in Rust using the shrs framework. It integrates vincents-ai/llm for intelligent command generation/tutoring and vincents-ai/gitoxide for state-safety. The project serves two distinct user personas through dynamic execution profiles: an educational "Kids Sandbox" and a strict "Agentic Framework" environment.

## **2\. Target Personas & Use Cases**

### **2.1 Persona A: The Learner (Ages 5-9)**

* **Goal:** Learn terminal navigation and basic computer interaction without the risk of destroying the host OS.  
* **Interaction Model:** Visual, gamified, and forgiving. AI acts as a tutor rather than an executor.  
* **Key Constraints:** \* Cannot navigate outside of designated sandbox directories (chroot/jail equivalent).  
  * Strict allowlist of benign commands (ls, cd, echo, cowsay).

### **2.2 Persona B: The AI Coding Agent**

* **Goal:** Execute multi-step coding, building, and deployment tasks within a structured, parseable environment.  
* **Interaction Model:** Headless or embedded. High volume of commands. Outputs must be structured (JSON).  
* **Key Constraints:**  
  * Must be able to modify the filesystem, but destructive actions must be reversible.  
  * Blocklist of recursive deletion or system-level network exfiltration.

## **3\. Functional Requirements**

### **3.1 Profile Initialization Layer**

* **REQ-3.1.1:** The binary must accept command-line arguments (e.g., \--mode kids, \--mode agent) to establish the runtime profile before shell initialization.  
* **REQ-3.1.2:** The Agent mode must enforce JSON serialization for all standard output, standard error, and exit codes.

### **3.2 Access Control List (ACL) Middleware**

* **REQ-3.2.1:** Every command (user-typed or AI-generated) must pass through a unified ACL parser.  
* **REQ-3.2.2:** The ACL must support both an Allowlist (explicit inclusion) and a Blocklist (explicit exclusion, overrides allowlist).  
* **REQ-3.2.3:** Blocked commands must not spawn an OS process. They must immediately return an access violation error.

### **3.3 LLM Integration (vincents-ai/llm)**

* **REQ-3.3.1:** The shell will expose a custom built-in command ? (or ai) to interface with the LLM.  
* **REQ-3.3.2:** In Kids Mode, the LLM prompt will be heavily seeded with a system prompt instructing it to act as an encouraging tutor, returning instructional text rather than executing.  
* **REQ-3.3.3:** In Agent Mode, the LLM is expected to generate POSIX execution strings formatted via a predefined JSON schema.

### **3.4 State Safety via Gitoxide (vincents-ai/gitoxide)**

* **REQ-3.4.1:** The shell must hook into shrs BeforeCommandCtx and AfterCommandCtx.  
* **REQ-3.4.2:** Prior to the execution of any command flagged as "mutating" (e.g., rm, mv, cargo), the shell must query gix to check if CWD is within a valid git repository.  
* **REQ-3.4.3:** If in a repository, the shell must create an atomic "Pre-Execution Snapshot" commit.  
* **REQ-3.4.4:** Upon completion of the command, a "Post-Execution Snapshot" commit must be created, capturing the exit code in the commit message.

## **4\. Non-Functional Requirements**

* **Performance:** The shell input loop must maintain \<10ms latency. gix synchronous hooks must not block the main thread for more than 100ms.  
* **Portability:** Must compile on x86\_64 and aarch64 Linux/macOS targets via Nix.

## **5\. Development Roadmap**

* **Phase 1 (Foundation):** CLI parser, shrs basic configuration, and hardcoded ACL lists.  
* **Phase 2 (Safety):** Integration of gix hooks and snapshotting logic.  
* **Phase 3 (Intelligence):** Wiring of vincents-ai/llm with dynamic system prompting based on the active profile.