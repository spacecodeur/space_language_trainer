---
title: 'Personal Profile Context for Self-Introduction Practice'
slug: 'personal-profile-context'
created: '2026-02-26'
status: 'ready-for-dev'
stepsCompleted: [1, 2, 3, 4]
tech_stack: ['markdown (agent prompt)', 'claude CLI (--system-prompt)']
files_to_modify:
  - 'agent/language_trainer.agent.md'
code_patterns: ['## section headers for agent instructions', 'plain English prose (no markdown in responses, but markdown in system prompt is fine)']
test_patterns: ['manual E2E voice session testing only']
---

# Tech-Spec: Personal Profile Context for Self-Introduction Practice

**Created:** 2026-02-26

## Overview

### Problem Statement

The language trainer agent has no knowledge of the user's professional background, skills, education, or career history. This makes it impossible to practice self-introduction in English realistically — Claude cannot verify factual accuracy, suggest better structuring of professional experience, or correct domain-specific vocabulary related to the user's actual career. The user needs to practice introductions in multiple contexts: formal job interviews, semi-formal networking, and casual social encounters.

### Solution

Synthesize all 3 CVs (LinkedIn export, post-Wild Code School CV, detailed visual CV) into a comprehensive English-language user profile and inject it as a dedicated "User Profile" section in the `language_trainer.agent.md` agent prompt file. The profile includes career data for formal contexts AND personal interests/hobbies for casual contexts. Zero Rust code changes required.

### Scope

**In Scope:**
- Analyze and cross-reference all 3 PDF CVs to extract the most complete and accurate information
- Synthesize into an exhaustive English-language profile covering: identity, professional summary, complete career timeline, education, certifications, technical skills, soft skills, hobbies/interests
- Add a "User Profile" section to `agent/language_trainer.agent.md` with usage guidance covering three introduction contexts (job interview, networking, casual social)
- Include guidance for Claude on how to use the profile during conversations

**Out of Scope:**
- No new dedicated scenario mode (existing interview simulation + free conversation suffice)
- No Rust code changes in orchestrator, server, or client
- No file concatenation mechanism or separate profile file loading
- No dynamic profile updating at runtime

## Context for Development

### Codebase Patterns

The agent file is a plain markdown file loaded in its entirety as `--system-prompt`. It uses section headers (##) to organize instructions. The current section order is:

1. `## Voice Output Format — CRITICAL`
2. `## Core Persona`
3. `## CEFR-Aware Methodology`
4. `## Real-Time Correction Approach`
5. `## Feedback Modes`
6. `## Scenario Handling` (includes: Free Conversation, Grammar Drills, Interview Simulation, Topic Discussion, Level Assessment)
7. `## Conversation Flow Guidelines`
8. `## Session Sustainability`
9. `## Boundaries`
10. `## Language Feedback Display — SYSTEM CONTROL`
11. `## Speech Speed Control — MANDATORY`
12. `## Context Compaction — CRITICAL`
13. `## Final Reminder`

The new `## User Profile` section will be inserted between `## Core Persona` (section 2) and `## CEFR-Aware Methodology` (section 3).

### Files to Reference

| File | Purpose |
| ---- | ------- |
| `agent/language_trainer.agent.md` | Agent prompt file — target for profile injection |
| `docs/cv_linkedin.pdf` | LinkedIn profile export (3 pages) |
| `docs/CV_2025_post_wild.pdf` | Post-Wild Code School CV (3 pages) |
| `docs/LOPEZ_MATTHIEU_CV.pdf` | Detailed visual CV with project descriptions (5 pages) |
| `orchestrator/src/claude.rs` | Claude CLI backend — reads agent file, no changes needed |

### Technical Decisions

- **Direct injection into agent file**: Chosen over separate file concatenation for simplicity.
- **Profile in English**: The agent operates entirely in English. Source CVs are in French — translation required.
- **Exhaustive synthesis**: Cross-reference all 3 CVs to capture the most complete picture.
- **Section placement**: After "Core Persona", before "CEFR-Aware Methodology".
- **Token budget**: Current agent file ~4000 words. Profile adds ~900 words. Well within limits.

## Implementation Plan

### Tasks

- [ ] Task 1: Add "User Profile" section to the agent prompt file
  - File: `agent/language_trainer.agent.md`
  - Action: Insert the section defined below between `## Core Persona` and `## CEFR-Aware Methodology`.
  - Notes: Markdown formatting is allowed in the system prompt — the "no markdown" rule only applies to Claude's spoken responses.

#### Section Content to Insert

```markdown
## User Profile

The following is factual information about the user you are tutoring. Use this knowledge to help the user practice introductions and talk about themselves in English. Adapt your use of this profile to the context:

- **Job interview practice:** Help the user structure a professional self-introduction highlighting relevant experience, skills, and achievements. Coach on formal register, industry vocabulary, and concise delivery. Gently correct factual inaccuracies (wrong dates, wrong job titles).
- **Networking / elevator pitch:** Help the user craft a punchy, semi-formal introduction covering who they are, what they do, and what makes them interesting — in 30-60 seconds.
- **Casual social encounters:** Help the user introduce themselves naturally to strangers, new acquaintances, or friends of friends. Focus on personality, hobbies, interests, and conversational warmth rather than career details. Only mention work if it comes up naturally.

IMPORTANT: Never volunteer this information unprompted. Only use it when the user is practicing self-introduction, doing an interview simulation, or discussing their own background. During unrelated conversations, ignore this section entirely.

FORMATTING: When referencing profile data in your spoken responses, always use plain conversational English. Never echo the markdown formatting (bold, bullets, headers) from this section.

### Identity

- **Name:** Matthieu Lopez
- **Date of birth:** May 28, 1986
- **Location:** Lyon, France
- **Languages:** French (native), English (oral A2/B1, written B2/C1)

### Professional Summary

Senior software engineer and pedagogical engineer with over 10 years of experience spanning web development, QA engineering, and IT training. Holds the CAFEP certification (teaching credential for private education) in Computer Science, ranked 3rd nationally out of 75 candidates. Expert in instructional design and technical knowledge transfer. Independent consultant since 2018 under the brand "spacecodeur." Has worked with training organizations, private companies, and French government institutions (Ministry of Labor, Ministry of Foreign Affairs, Ministry of Armed Forces). Officially certified jury member for national professional certifications in web development (DWWM — Bac+2 level) and application design (CDA — Bac+3/+5 level).

### Career Timeline

**Senior Trainer & Tech Lead** — Wild Code School, Lyon (September 2023 – July 2025)
Led intensive "Web and Mobile Web Developer" training programs. Designed innovative pedagogical approaches including flipped classroom and project-based learning. Organized national Masterclass events on development best practices and automated testing. Mentored between 5 and 21 learners per session with varied profiles.

**QA Engineer & Senior Developer** — CNRS, Lyon (January 2023 – July 2023)
Worked on the HALiance project — a 5-year national initiative to rebuild the HAL scientific publication platform. Migrated legacy code to Symfony 6 in collaboration with SensioLabs (the company behind Symfony). Set up industrialization processes and automated testing. Trained teams on new architectures. Stack: Symfony, PHP, JavaScript, Docker, SQL, API REST, CI, Agile SCRUM.

**IT Trainer** — EPSI, Lyon (September 2022 – January 2023)
Delivered IT training internally and at client sites. Taught advanced Linux, Spring Boot with API REST, Python (OOP), and SQL database design. Continued developing "Edusophie," a personal interactive course platform.

**Tenured NSI Teacher** — Éducation Nationale, Lycée Beauséjour, Narbonne (September 2021 – August 2022)
Taught Computer Science in high school after passing the CAFEP national exam (3rd out of 75). Spent half the year in formal teacher training at INSPE, acquiring expertise in pedagogy and didactics. Built "Edusophie" — an interactive course platform using markdown/HTML files in git repositories with a custom widget system.

**QA Manager & Test Architect** — SEPTEO, Montpellier (April 2020 – August 2020)
Set up test architectures (Selenium, Cypress, Codeception). Trained business and technical teams on testing methodologies.

**University Lecturer** — Université Montpellier 2 (November 2019 – January 2020)
Taught "Advanced Web Technologies" at Master 2 level: Symfony v5, SQL databases, Git, CI/CD.

**QA Lead Engineer** — Sogétrel, Montpellier (August 2018 – September 2019)
Designed and built "Symepty," a SaaS platform for automated testing (Symfony 4). Set up test architectures and trained teams on testing methodologies.

**IT & Pedagogical Consultant** — Spacecodeur (self-employed), France (2018 – present)
Full-stack web development, QA consulting, RNCP jury duties, pedagogical strategy design for training organizations. Clients include training organizations, private companies, and French government ministries.

**Web/IT Trainer** — Groupe AFORMAC, Limousin (June 2017 – July 2018)
Led two 7-month training programs for the DWWM professional title. Mentored learners during internships.

**QA Engineer, System & PHP/Drupal Tech Lead** — DOCAPOST (La Poste Group subsidiary), Sophia Antipolis (July 2015 – April 2017)
Initially hired to take over Drupal site development, then joined the "Yellowstone Gamme RH" project — a modular SaaS HR solution for medium and large companies. Participated in technical design phases and qualified versions produced by the outsourcing partner AUSY (manual and automated tests). Prepared the production environment (load balancing, SSO authentication). After delivering a stable first version, helped build an internal development team: participated in recruitment, product training, and ongoing development. Alternated between QA and Drupal reference developer roles. Stack: Drupal, PHP, PostgreSQL, JavaScript, Selenium IDE, CasperJS, JMeter, Linux, Bash, Git.

**QA Lead Tester** — StarDust (digital testing startup), Marseille area (February 2014 – July 2015)
StarDust specialized in functional testing of web and mobile applications across predefined device sets (smartphones, tablets, desktop) and operating systems (Windows, Mac, Android, iOS). Initially hired as a tester, technical background led to quickly becoming lead of teams of 2 to 10 testers across 30+ projects. Clients included CMA, Channel, 3 Suisses, and FDJ (French national lottery). Developed automated tests with CasperJS. Refactored Excel/VBA reporting scripts, reducing execution time from several minutes to seconds. Led the complete redesign of the internal "Scapera" device stock management application using Drupal. Stack: Drupal, PHP, JavaScript, SQL, CasperJS, Excel VBA, Linux, Bash, Git.

**Web Integration & Development Engineer** — Smile (open-source solutions ESN), Marseille (April 2013 – October 2013)
Joined Smile for their expertise in open-source CMS, having discovered Drupal during university studies. Worked on multiple client projects: EDF-Hermes (Drupal 5 to 7 migration), Cultura (functional testing, Drupal administration), Action contre la Faim (custom Drupal 7 modules, external content retrieval), Vectis Conseil (from-scratch Drupal Commerce site migration, responsive front-end, English and Flemish translations, production deployment), and Première Vision (WordPress front-end redesign). Stack: Drupal, WordPress, PHP, JavaScript, CSS/LESS, Bootstrap, responsive design, SQL, API REST, Linux, Bash.

### Education

- **Master 2 in Computer Science** — Distributed Information Systems, Aix-Marseille Université (2012–2014). Valedictorian (1st out of 15 in final year).
- **CAFEP in Computer Science** — National teaching credential, Éducation Nationale (2021). Ranked 3rd nationally out of 75.
- **Advanced Pedagogical Engineering** — INSPE (2020–2021). Formal training in pedagogy and didactics.
- **Data Scientist Path** — OpenClassrooms (2018).
- **English Language Training** — 6-month program focused on listening and speaking skills (2018).
- **Bachelor's in Mathematics and Computer Science** — Web specialization, Aix-Marseille Université (2011).
- **Scientific Baccalauréat** — Mathematics specialization, Lycée Félix Esclangon, Manosque (2006).

### Technical Skills

**Core (8+ years):** PHP, SQL, Linux, Bash
**Strong (5-7 years):** JavaScript (ES6+), Drupal
**Solid (3-4 years):** Symfony, SASS, Selenium, API REST
**Working (2+ years):** Python, Cypress, Rust, Docker, React, Node.js, Express, TypeScript, TDD, PHPUnit
**Additional knowledge:** AWS, Flask, GitHub Actions, JMeter, Kali Linux, ML basics, MongoDB, Spring Boot, Angular, GitLab CI

### Soft Skills & Professional Strengths

Training facilitation, instructional design, code review leadership, developer mentoring, quality process implementation, Agile project management, technical documentation, technology watch, trainer-of-trainers programs.

### Notable Projects

- **Edusophie** — Custom interactive course platform (markdown/HTML in git repos, widget system). Publicly accessible at edusophie.fr.
- **Symepty** — SaaS automated testing platform (Symfony 4), built at Sogétrel.
- **HALiance** — National scientific publication platform refactoring (CNRS, 5-year project with SensioLabs).

### Hobbies & Personal Interests

Lindy hop (swing dancing), playing chess, listening to music, board games and video games, reading (computer science, science fiction, comics). Following scientific and political conferences. Private tutoring and volunteering. Coding personal projects — recently exploring Rust programming and AI/LLM technologies.
```

- [ ] Task 2: Update Context Compaction section to preserve user profile
  - File: `agent/language_trainer.agent.md`
  - Action: In the existing `## Context Compaction — CRITICAL` section, add a bullet point instructing Claude to preserve the user profile data (name, career timeline, key facts) during context compaction, since it is static reference data needed throughout the session.
  - Notes: Without this, Claude may drop profile details during long sessions, causing it to "forget" career facts mid-interview practice.

- [ ] Task 3: Validate agent file integrity
  - File: `agent/language_trainer.agent.md`
  - Action: Verify the file is valid markdown and section structure is coherent. Check word count stays under 6000 words.

- [ ] Task 4: Manual E2E validation
  - Action: Launch a voice session and test at minimum:
    1. "Let's practice my self-introduction for a job interview" — verify Claude coaches formal introduction using career data
    2. "How would I introduce myself at a networking event?" — verify Claude helps craft a punchy elevator pitch
    3. "I'm meeting new people at a party, how do I introduce myself?" — verify Claude focuses on personality, hobbies, warmth rather than career details
    4. Deliberately misstate a fact (e.g., wrong dates) — verify Claude gently corrects
    5. Free conversation unrelated to career — verify Claude does NOT volunteer profile information

### Acceptance Criteria

- [ ] AC 1: Given the updated agent file, when the user says "let's practice my self-introduction for a job interview," then Claude coaches a formal, structured professional introduction using knowledge from the profile.

- [ ] AC 2: Given the updated agent file, when the user asks to practice introducing themselves at a networking event, then Claude helps craft a concise elevator pitch (30-60 seconds) balancing professional identity and personality.

- [ ] AC 3: Given the updated agent file, when the user asks to practice a casual social introduction (party, meeting strangers), then Claude focuses on personality, hobbies, and conversational warmth rather than listing career achievements.

- [ ] AC 4: Given the updated agent file, when the user misstates a fact about their career during practice, then Claude gently corrects the inaccuracy with the right information.

- [ ] AC 5: Given the updated agent file, when the user engages in free conversation unrelated to their career, then Claude does NOT volunteer profile information unprompted.

- [ ] AC 6: Given the updated agent file, when the orchestrator starts a new session, then the agent file loads without errors and existing features work normally (corrections, feedback modes, speed control — no regression).

## Additional Context

### Dependencies

None — pure prompt engineering change with no code dependencies.

### Testing Strategy

Manual E2E testing only:
1. Launch full stack (server + orchestrator + client)
2. Practice self-introduction in the three contexts (interview, networking, casual)
3. Deliberately test fact-checking by misstating career details
4. Verify no regression in existing agent behaviors

### Notes

- **Token budget**: The profile adds approximately 900 words. Total system prompt ~5000 words — well within limits.
- **Maintenance**: If the user's career changes, the profile section must be manually updated in the agent file.
- **Privacy**: The profile contains personal information. The agent file is local-only. Ensure repository access controls are appropriate if pushing to a remote.
