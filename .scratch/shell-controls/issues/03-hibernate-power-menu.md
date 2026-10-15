# Add Hibernate to the power menu

Status: resolved
Type: task

Add a confirmed Hibernate action to the power menu. Use logind's capability
readback to disable it when the system cannot hibernate.

## Comments

Added Hibernate using logind's `CanHibernate` capability and `Hibernate` method.
The menu requires an explicit in-menu confirmation and has additional vertical
space for the fifth action.

Menu sizing now uses explicit title and action line heights together with row
padding and gaps. Built and deployed the correction; the live menu screenshot
shows the complete Shut down row with bottom padding.
