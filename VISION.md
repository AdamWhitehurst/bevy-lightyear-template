# Game Vision Document

A multiplayer open-world game where players raise, breed, and train persistent characters. Exploring an overworld, competing in varied challenges, and
nurturing their creatures in a personal home-base.

## Core Fantasy

**Players become proud parents of unique creatures they've raised from scratch.**

Your brawlers aren't just avatars; they're creatures with personalities, genetics, alignments, and legacies. You care for them in your home-base, take
them into the overworld to explore and quest, challenge other players to duels, and compete in arenas, races, and other challenges. Every action
shapes who your brawlers become.

## Design Pillars

### 1. Living Home-Base

- Personal space where all your brawlers live together
- Control a floating "god hand" to interact: feed, pet, pick up, give toys
- Encourage brawlers to play together, form relationships, and mate
- Customize with furnishings, toys, and collectibles found in the world
- Watch brawlers interact autonomously when you're not directing them

### 2. Open-World Exploration

- Persistent overworld where players encounter each other
- Challenge others to duels on the spot or invite to instanced arenas
- Accept quests from NPCs or other players for rewards
- Collect items to bring back: furnishings, toys, food, exotic eggs
- Cooperative play in levels that permit it

### 3. Varied Challenges

- Not just fighting— multiple game modes testing different stats
- Pre-defined stages (arenas, landmarks) and randomly-generated ones
- Solo and multiplayer challenges with different objectives

### 4. Meaningful Progression

- Brawlers evolve through training, breeding, and moral choices
- Stats unlock abilities (stat requirements for moves)
- Appearance changes based on stat focus and alignment
- Multi-generational inheritance creates legacy

## Game Modes

### Combat

| Mode       | Description                                                           |
| ---------- | --------------------------------------------------------------------- |
| **Duels**  | 1v1 battles in the overworld, quick and informal                      |
| **Arenas** | Instanced competitive fights, pre-defined stages                      |
| **Sumo**   | Push opponent out of bounds; tests stamina/agility (weight) and power |

### Challenges

| Mode               | Description                                                   |
| ------------------ | ------------------------------------------------------------- |
| **Time Trials**    | Test agility and stamina on obstacle courses                  |
| **Racing**         | Multiplayer speedrunning through stages                       |
| **Collection**     | Gather items within time/space constraints                    |
| **Bounty Hunting** | Capture or kill wanted criminals/lost pets; affects alignment |

## Brawler Systems

### Stats & Abilities

- **Stat Grades** (E→S): Persist through generations, determine growth potential
- **Stat Requirements**: Abilities have minimum stat thresholds to equip
- **Training**: Stat focus through activities shapes brawler's strengths

### Alignment

- **Good/Evil Spectrum**: Shifts based on actions
  - Killing shifts toward evil
  - Capturing criminals shifts toward good
  - Bounty hunting choices have lasting moral consequences
- **Appearance Changes**: Alignment visually transforms brawlers over time

### Genetics & Breeding

- **Genotypes**: Stats inherited from parents with slight mutation
- **Phenotypes**: Appearance inherited with variation
- **Relationships**: Brawlers form bonds, leading to mating when encouraged
- **Offspring**: New brawlers carrying genetic legacy of their lineage

### Appearance Evolution

- Appearance changes based on:
  - Stat training (strength-focused → muscley; agility-focused → sleek)
  - Alignment (hue-shifting)
  - Inherited phenotypes from parents
  - Collected cosmetics/body parts

## World Structure

### Home-Base

- Private instance per player
- God-hand control mode for caretaking
- Brawlers roam and interact autonomously
- Editable: place furnishings, toys, decorations
- Portal/exit to overworld

### Overworld

- Shared persistent space
- Other players visible and interactable
- NPCs offering quests
- Entrances to instanced stages
- Admin-editable (world building tools)

### Instanced Stages

- **Pre-defined**: Arenas, story stages, landmarks (consistent layout)
- **Randomly Generated**: Exploration zones, some challenge types (replayability)
- Some stages allow cooperation; others are competitive

## Stage Editing

- Players can edit their home-base layout
- Admins can edit the overworld
- Tools for creating new pre-defined instances
- Potentially user-generated content for challenges

## Target Experience

**Session Flow:**

```
Launch → Visit home-base → Care for brawlers → Enter overworld
→ Quest/explore/duel → Enter challenge stage → Earn rewards
→ Return home with loot → Customize base → Repeat
```

**Long-term Loop:**

```
Raise brawlers → Train stats → Shape alignment → Breed offspring
→ Generations improve → Unlock better abilities → Climb ranks
```

## Key Mechanics Summary

| System          | Core Loop                                        |
| --------------- | ------------------------------------------------ |
| **Home-Base**   | Care → Relationships → Breeding → New brawlers   |
| **Overworld**   | Explore → Quest → Duel → Collect                 |
| **Challenges**  | Compete → Earn rewards → Improve brawlers        |
| **Progression** | Train → Evolve appearance → Inherit to offspring |

## Inspirations

| Source              | What We Take                                                             |
| ------------------- | ------------------------------------------------------------------------ |
| **Chao Garden**     | Home-base caretaking, stat grades, breeding, happiness, visual evolution |
| **Sonic Battle**    | Fast combat, move copying, special slots, comeback gauge                 |
| **Pokemon**         | Breeding, inheritance, type effectiveness, collection                    |
| **Animal Crossing** | Home customization, collecting furnishings                               |
| **Black & White**   | God-hand interaction, creature alignment                                 |

## Success Criteria

A player should feel:

- **Attachment** to brawlers they've raised and bred across generations
- **Pride** in their home-base and unique creature builds
- **Agency** in shaping alignment through moral choices
- **Variety** from different challenge types beyond just fighting
- **Community** through overworld encounters and player quests

## Design Decisions

1. **Lifespan**: Activity-based (~80% from playing with that brawler, ~20% from overall session time). Idle aging only accrues while player is online,
   not logged out.
2. **Permadeath**: Optional "hardcore" mode—leave flexibility for both retirement and deletion.
3. **Economy**: Items tradeable; brawlers are not.
4. **Monetization**: Cosmetic-only, no progression shortcuts.
