The goal is:

* Few enough stats to be readable
* Deep enough that specialization matters
* Broad enough to power races, sumo, bounties, care systems, and breeding

---

## Core Stat Categories (High-Level)

Brawlers have **3 layers of stats**:

1. **Primary Physical Stats** – define how the brawler plays
2. **Derived Combat Stats** – calculated, not trained directly
3. **Behavioral / Meta Stats** – affect AI, care, and world interactions

Players mostly train **Primary Stats**. Everything else flows from them.

---

## 1. Primary Physical Stats (Trainable & Inheritable)

These are the *genetic backbone* of a brawler.

### **1. Power**

**What it represents:** Raw strength and force output

**Affects:**

* Damage dealt
* Knockback force
* Throw strength
* Sumo push effectiveness
* Environmental interaction (breaking objects, lifting weight)

**Ability Requirements:**

* Heavy strikes
* Launchers
* Grab-based specials

**Visual Impact:**

* Bulkier limbs
* Thicker torso
* More aggressive animations

---

### **2. Agility**

**What it represents:** Speed, reflexes, coordination

**Affects:**

* Run speed
* Air control
* Dodge recovery
* Combo execution windows
* Effective “lightness” in sumo (evasion)

**Ability Requirements:**

* Multi-hit strings
* Teleports / dashes
* Aerial-heavy kits

**Visual Impact:**

* Leaner silhouette
* Longer limbs
* Snappier movement

---

### **3. Stamina**

**What it represents:** Endurance and physical resilience

**Affects:**

* Max stamina meter
* Stamina regeneration
* Resistance to knockback
* Time before exhaustion in races
* Sumo balance and footing

**Ability Requirements:**

* Charge attacks
* Sustained buffs
* Guard-heavy playstyles

**Visual Impact:**

* Broader stance
* Denser body
* Slower breathing animations

---

### **4. Focus**

**What it represents:** Mental discipline and precision

**Affects:**

* Special move accuracy
* Comeback gauge efficiency
* Perfect block window
* Status effect resistance
* Capture success (non-lethal bounties)

**Ability Requirements:**

* Counter moves
* Precision projectiles
* Technical control abilities

**Visual Impact:**

* Subtle glow in eyes
* Controlled idle animations
* Refined posture

---

### **5. Vitality**

**What it represents:** Life force and physical robustness

**Affects:**

* Max health
* Injury resistance
* Survival chance in lethal encounters
* Aging rate (high vitality = longer lifespan)

**Ability Requirements:**

* Regeneration
* Sacrifice-based abilities
* Survival passives

**Visual Impact:**

* Healthier coloration
* Scars fade slower
* Fuller proportions

---

## 2. Derived Combat Stats (Calculated)

These are *never trained directly*—they emerge from primary stats.

| Derived Stat       | Comes From         | Purpose            |
| ------------------ | ------------------ | ------------------ |
| Damage             | Power + Focus      | Raw output         |
| Knockback          | Power + Stamina    | Arena control      |
| Speed              | Agility            | Movement           |
| Balance            | Stamina + Agility  | Sumo, edge control |
| Special Efficiency | Focus              | Meter usage        |
| Survivability      | Vitality + Stamina | Staying power      |

This keeps the system **understandable** while still deep.

---

## 3. Behavioral & Meta Stats (Care-Driven)

These stats grow through **treatment and choices**, not combat.

### **6. Happiness**

* Influenced by care, victories, relationships
* Low happiness:

  * Shortened lifespan
  * Worse AI decisions
  * Risk of permanent death
* High happiness:

  * Faster learning
  * Slight stat growth bonuses
  * Better offspring outcomes

---

### **7. Loyalty**

* How strongly the brawler trusts the player
* Affects:

  * Obedience in AI-controlled modes
  * Performance consistency
  * Willingness to take risks for the player

Neglect reduces loyalty. Abuse destroys it.

---

### **8. Aggression**

* How prone the brawler is to lethal force
* Influences:

  * AI combat style
  * Alignment drift
  * Bounty outcomes (kill vs capture bias)

Aggression is *not* evil—but it nudges that direction.

---

## 4. Alignment (Good ↔ Evil Axis)

Alignment affects:

* Visual mutations
* Ability access
* NPC reactions
* Offspring tendencies

---

## 5. Breeding & Inheritance Model

Each brawler has:

* **Stat Grades (E → S)** – inherited strongly
* **Stat Values** – averaged + mutated
* **Behavioral tendencies** – probabilistic inheritance

Example:

```
Parent A: High Agility (A), Low Power (D)
Parent B: Medium Agility (B), High Power (A)

Offspring:
Agility Grade: B or A
Power Grade: B or A
Small mutation chance
```

Happiness and alignment influence mutation quality.

---

## 6. Why This Stat Set Works

✔ Supports **combat, races, sumo, bounties, care**
✔ Enables **specialization without hard classes**
✔ Naturally feeds into **appearance changes**
✔ Scales across **generations**
✔ Easy to explain, hard to master

---

## Optional Advanced Stats (Late-Game / Rare)

If you want deeper builds later:

* **Mass** – explicit weight (affects sumo & knockback)
* **Instinct** – reaction speed, prediction
* **Presence** – social influence in overworld

These should be *rare traits*, not default stats.

---

## TL;DR – Recommended Core Stats

**Primary (Trainable):**

* Power
* Agility
* Stamina
* Focus
* Vitality

**Care / Meta:**

* Happiness
* Loyalty
* Aggression

**Alignment:**

* Good ↔ Evil (emergent modifier)

If you want, next I can:

* Map stats to **exact numeric ranges**
* Design **training activities per stat**
* Show **example brawler archetypes**
* Define **ability requirement thresholds**
* Tie stats to **procedural level generation**

Just tell me which layer you want to go deeper on.