# AgentLens Pricing & Business Model

## Overview

AgentLens uses a freemium model with bottom-up adoption and top-down enterprise sales. The local agent is free to maximize distribution; collaboration and compliance features are paid.

---

## Pricing Tiers

### Free

**$0 / forever**

For individual developers exploring AI coding provenance.

| Feature | Included |
|---------|----------|
| Local agent (Claude Code) | ✓ |
| Local session capture | ✓ |
| Local SQLite storage | ✓ |
| CLI session viewer | ✓ |
| Local web UI (localhost) | ✓ |
| Git commit linking (local) | ✓ |
| Session retention | Unlimited (local) |

**Purpose**: Distribution engine. Get developers hooked on seeing their own sessions.

---

### Team

**$20 / active seat / month**

For teams that want to share session context during code review.

| Feature | Included |
|---------|----------|
| Everything in Free | ✓ |
| Cloud sync | ✓ |
| GitHub/GitLab PR integration | ✓ |
| PR comments with session links | ✓ |
| Session sharing (shareable links) | ✓ |
| Team dashboard | ✓ |
| Basic analytics | ✓ |
| Session retention | 90 days |
| Support | Email |

**"Active seat"** = User who opened dashboard OR synced a session in the last 30 days. Inactive users don't count toward billing.

**Minimum**: 3 seats

---

### Enterprise

**$40 / seat / month**

For organizations requiring compliance, governance, and advanced controls.

| Feature | Included |
|---------|----------|
| Everything in Team | ✓ |
| SSO/SAML | ✓ |
| SCIM provisioning | ✓ |
| Role-based access control | ✓ |
| Custom retention policies | ✓ |
| Legal hold | ✓ |
| Audit log export | ✓ |
| Compliance reporting (SOC 2) | ✓ |
| Struggle detection signals | ✓ |
| Leadership analytics dashboard | ✓ |
| On-prem deployment option | ✓ |
| Session retention | Unlimited / custom |
| Support | Dedicated CSM, SLA |

**Minimum**: 25 seats (annual contract)

**Volume discounts**:
- 100+ seats: 15% discount
- 250+ seats: 25% discount
- 500+ seats: Custom pricing

---

## Pricing Rationale

### Competitive Benchmarks

| Tool | Category | Pricing |
|------|----------|---------|
| GitHub Copilot | AI coding | $19/seat/mo |
| GitHub Copilot Enterprise | AI coding + governance | $39/seat/mo |
| GitClear | Code analytics | ~$15/seat/mo |
| CodeScene | Tech debt analysis | ~$30/seat/mo |
| LinearB | Dev productivity | ~$40/seat/mo |
| Pluralsight Flow | Engineering intelligence | ~$40/seat/mo |

AgentLens sits in the **"engineering intelligence"** category, not the AI coding tool category. $20-40/seat is the appropriate range.

### Why Per-Seat (Not Per-Repo or Usage-Based)

| Model | Pros | Cons |
|-------|------|------|
| Per-seat | Predictable revenue, scales with org size | Can feel like "tax" on adoption |
| Per-repo | Aligns with project growth | Punishes monorepos, hard to predict |
| Usage-based | Pure value alignment | Unpredictable for customer budgets |

**Decision**: Per-seat with "active user" definition removes shelfware concerns while maintaining predictability.

### Why "Active Seat" Billing

Traditional per-seat licensing charges for every provisioned user. This creates friction:
- "Do we really need to add this intern?"
- "Let's wait until next quarter to onboard the new team"

Active seat billing means:
- Only users who actually used AgentLens in the last 30 days are billed
- Customers feel safe rolling out broadly
- Revenue still scales with actual adoption

---

## Gating Strategy

### What's Free vs Paid

```
┌─────────────────────────────────────────────────────────────────┐
│                            FREE                                 │
│                   (Local, Single Developer)                     │
├─────────────────────────────────────────────────────────────────┤
│  ✓ Session capture                                              │
│  ✓ Local storage                                                │
│  ✓ Local replay UI                                              │
│  ✓ Git commit linking                                           │
│  ✓ CLI tools                                                    │
└─────────────────────────────────────────────────────────────────┘
                              │
                    ──────────┼────────── Paywall
                              │
┌─────────────────────────────────────────────────────────────────┐
│                            TEAM                                 │
│                   (Cloud, Collaboration)                        │
├─────────────────────────────────────────────────────────────────┤
│  ✓ Cloud sync (sessions accessible from any device)             │
│  ✓ Session sharing (send link to reviewer)                      │
│  ✓ GitHub/GitLab PR integration                                 │
│  ✓ Team dashboard                                               │
└─────────────────────────────────────────────────────────────────┘
                              │
                    ──────────┼────────── Paywall
                              │
┌─────────────────────────────────────────────────────────────────┐
│                         ENTERPRISE                              │
│                   (Compliance, Governance)                      │
├─────────────────────────────────────────────────────────────────┤
│  ✓ SSO/SAML                                                     │
│  ✓ RBAC                                                         │
│  ✓ Audit export                                                 │
│  ✓ Compliance reporting                                         │
│  ✓ Advanced analytics                                           │
│  ✓ On-prem option                                               │
└─────────────────────────────────────────────────────────────────┘
```

### Gating Philosophy

**Free tier = distribution**. Don't gate anything that prevents a developer from experiencing the core value (seeing their own sessions).

**Team tier = collaboration**. The moment value involves another person (sharing, PR integration), that's the paywall.

**Enterprise tier = compliance**. SSO is table stakes for enterprise procurement. Everything else is about satisfying security/legal requirements.

---

## Conversion Triggers

Build these moments into the product to prompt upgrades:

### Free → Team

| Trigger | Context | Prompt |
|---------|---------|--------|
| First shareable moment | User tries to copy session link | "Share this session with your reviewer? Upgrade to Team." |
| PR opened with AI code | Commit linked to session, PR created | "Add AgentLens context to this PR? Upgrade to Team." |
| Second device | User logs in from new machine | "Sync your sessions across devices? Upgrade to Team." |
| Team growth | 3+ users from same email domain | "Manage your team together? Upgrade to Team." |

### Team → Enterprise

| Trigger | Context | Prompt |
|---------|---------|--------|
| SSO request | Admin tries to configure SAML | "SSO requires Enterprise. Talk to sales." |
| Retention limit | Session approaches 90-day expiry | "Keep sessions longer? Upgrade to Enterprise." |
| Export request | User tries to export audit log | "Audit export requires Enterprise. Talk to sales." |
| Headcount growth | Team exceeds 20 active users | "Get volume discounts with Enterprise. Talk to sales." |

---

## Go-to-Market Motion

### Phase 1: Bottom-Up Adoption

```
Developer discovers AgentLens
         │
         ▼
Installs free local agent
         │
         ▼
Uses for personal workflow (sees value)
         │
         ▼
Shows teammate during code review
         │
         ▼
Teammate installs
         │
         ▼
Team wants to share sessions → TEAM UPGRADE
```

### Phase 2: Top-Down Enterprise Sale

```
VP Eng / CISO hears about AgentLens
         │
         ▼
Discovers team already using it (organic adoption)
         │
         ▼
Requests enterprise features (SSO, compliance)
         │
         ▼
Sales conversation
         │
         ▼
Enterprise contract → ENTERPRISE UPGRADE
```

### Key Insight

The best enterprise deals start with organic adoption. By the time procurement is involved, the tool is already in use and valued by developers. This inverts the typical enterprise sales motion and shortens deal cycles.

---

## Revenue Projections

### Year 1 (Conservative)

| Metric | Assumption |
|--------|------------|
| Free installs | 10,000 |
| Free → Team conversion | 5% |
| Team seats | 500 |
| Avg Team size | 5 |
| Team accounts | 100 |
| Team → Enterprise conversion | 10% |
| Enterprise seats | 250 |
| Enterprise accounts | 10 |

**Monthly Revenue**:
- Team: 500 seats × $20 = $10,000
- Enterprise: 250 seats × $40 = $10,000
- **Total MRR: $20,000**
- **ARR: $240,000**

### Year 2 (Growth)

| Metric | Assumption |
|--------|------------|
| Free installs | 50,000 |
| Team seats | 2,500 |
| Enterprise seats | 2,000 |

**Monthly Revenue**:
- Team: 2,500 × $20 = $50,000
- Enterprise: 2,000 × $40 = $80,000
- **Total MRR: $130,000**
- **ARR: $1.56M**

### Year 3 (Scale)

| Metric | Assumption |
|--------|------------|
| Free installs | 200,000 |
| Team seats | 8,000 |
| Enterprise seats | 10,000 |

**Monthly Revenue**:
- Team: 8,000 × $20 = $160,000
- Enterprise: 10,000 × $40 = $400,000
- **Total MRR: $560,000**
- **ARR: $6.7M**

---

## Expansion Revenue

### Within-Account Growth

Enterprise accounts grow as:
1. More developers adopt AI coding tools
2. More teams within org roll out AgentLens
3. Usage mandates expand coverage

**Net Revenue Retention target**: 120%+ (accounts grow faster than churn)

### Potential Add-Ons (Future)

| Add-On | Price | Target |
|--------|-------|--------|
| Extended retention (beyond 90 days) | $5/seat/mo | Team accounts wanting history |
| Advanced signals (custom rules) | $10/seat/mo | Enterprise accounts |
| On-prem support contract | $50k/year | Regulated industries |
| API access (integrate with internal tools) | $10/seat/mo | Platform teams |

---

## Sales Model

### Self-Serve (Free + Team)

- No sales involvement
- Credit card checkout
- In-app upgrade flows
- Support via docs + email

### Sales-Assisted (Enterprise)

- Inbound from Team accounts hitting limits
- Outbound to orgs with 10+ free users
- Demo → POC → Contract
- Annual contracts, invoiced

### Enterprise Deal Characteristics

| Metric | Target |
|--------|--------|
| Average Contract Value | $50,000 - $200,000 |
| Sales cycle | 30-60 days (short due to existing adoption) |
| POC duration | 2 weeks |
| Typical first contract | 50-100 seats |
| Expansion within 12 months | 2-3x initial seats |

---

## Pricing Objections & Responses

### "Why pay when I can see sessions locally for free?"

> "Free gives you personal visibility. Team lets you share that visibility with reviewers—which is where the real value is. When a reviewer can see *why* code looks the way it does, reviews go faster and merge with more confidence."

### "We already pay for GitHub Copilot / Cursor"

> "Those help you *write* code. AgentLens helps you *review* code. They're complementary. Copilot doesn't show reviewers what happened before the PR—AgentLens does."

### "$40/seat is expensive"

> "That's less than one hour of senior engineer time per month. If AgentLens saves your reviewers 30 minutes per week by giving them context they'd otherwise have to ask for, it pays for itself 5x over."

### "Can we get a discount?"

> "Volume discounts start at 100 seats (15% off). For 250+ we can do 25%. Let's talk about your expected rollout plan."

---

## Metrics to Track

### Acquisition

- Free installs (total, weekly)
- Install → First session captured (activation)
- Organic vs paid acquisition

### Engagement

- Weekly active users (WAU)
- Sessions captured per user per week
- Replay views per user
- PR comments generated

### Conversion

- Free → Team conversion rate
- Time to convert (days from install)
- Team → Enterprise conversion rate
- Upgrade trigger attribution (which prompt worked)

### Revenue

- MRR / ARR
- Net Revenue Retention (NRR)
- Average Revenue Per Account (ARPA)
- Customer Acquisition Cost (CAC)
- Lifetime Value (LTV)
- LTV:CAC ratio (target: >3:1)

### Churn

- Logo churn (accounts lost)
- Revenue churn (MRR lost)
- Reasons for churn (survey on cancel)

---

## Summary

| Tier | Price | Target | Key Gate |
|------|-------|--------|----------|
| Free | $0 | Individual devs | Local only |
| Team | $20/active seat/mo | Teams doing code review | Cloud sync, sharing, PR integration |
| Enterprise | $40/seat/mo | Orgs needing compliance | SSO, RBAC, audit, on-prem |

**Model**: Freemium → Self-serve Team → Sales-assisted Enterprise

**Motion**: Bottom-up adoption creates demand; top-down closes enterprise deals

**Key insight**: Don't charge for visibility into your own work. Charge for sharing that visibility with others.
