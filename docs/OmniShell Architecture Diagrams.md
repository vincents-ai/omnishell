# **OmniShell Architecture**

## **1\. Execution Pipeline**

flowchart TD  
    Input(\[User or Agent Input\]) \--\> Parser\[shrs Core Parser\]  
      
    subgraph OmniShell Security Boundary  
        Parser \--\> ACL\_Check{ACL Engine}  
        ACL\_Check \-- "Permitted" \--\> Mode\_Check{Execution Mode}  
        ACL\_Check \-- "Blocked" \--\> Output\_Err\[Return Security Violation\]  
    end  
      
    subgraph Gitoxide State Manager  
        Mode\_Check \-- Mutating Command \--\> Gix\_Pre\[gix: Pre-Exec Commit\]  
        Gix\_Pre \--\> OS\_Exec  
    end  
      
    Mode\_Check \-- Read-Only Command \--\> OS\_Exec\[OS Process Execution\]  
      
    OS\_Exec \--\> Gix\_Post\[gix: Post-Exec Commit\]  
      
    Gix\_Post \--\> Format\_Check{Is Agent Mode?}  
      
    Format\_Check \-- Yes \--\> JSON\[Format stdout/stderr to JSON\]  
    Format\_Check \-- No \--\> Standard\[Standard Terminal Output\]  
      
    JSON \--\> Term\_Out(\[Stdout\])  
    Standard \--\> Term\_Out  
    Output\_Err \--\> Term\_Out

## **2\. LLM Tutor Flow (Kids Mode)**

sequenceDiagram  
    participant Kid  
    participant OmniShell  
    participant vincents\_ai\_llm  
      
    Kid-\>\>OmniShell: \`? how do I make a file\`  
    OmniShell-\>\>vincents\_ai\_llm: System: "Act as a 5-yr-old tutor."\\nUser: "how do I make a file"  
    vincents\_ai\_llm--\>\>OmniShell: Response: "Use the \`touch\` command\!"  
    OmniShell-\>\>Kid: 🤖 Use the \`touch\` command\! Try typing: \`touch my\_file.txt\`  
