# Jump as Ability + Grounded Marker + CastConditions

Refactor jumping out of the movement-input special case and into a first-class ability in the existing ability system. Extract ground detection into its own system that maintains an `IsGrounded` sparse-set marker component, scheduled before movement input. Introduce a data-driven `CastConditions` mechanism (e.g. a component on ability definitions) so abilities can declaratively require conditions such as "only if grounded" without bespoke checks in each ability's activation code.
