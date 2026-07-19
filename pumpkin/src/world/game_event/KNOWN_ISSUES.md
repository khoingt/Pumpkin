# Known Issues — Sculk Sensor / Game Event System

## Pre-existing: Wolf Entity Data Kick (unrelated to sculk sensor)

**Date:** 2026-07-19  
**Severity:** Client crash (kick)  
**Status:** Pre-existing, out of scope for sculk sensor port

### Symptom
Client disconnects with "Network Protocol Error" when loading a Wolf entity. The error is:
```
Invalid entity data item type for field 18 on entity Wolf: old=0(class java.lang.Byte),
new=Reference{...WolfVariant...}(class net.minecraft.core.Holder$Reference)
```

### Cause
The server sends a `WolfVariant` holder reference for Wolf synched entity data field 18,
but the vanilla 26.2 client expects a `Byte`. The Wolf entity metadata type was changed
between Minecraft versions and Pumpkin hasn't caught up.

### Reproduction
- Join a world where a Wolf entity exists near the player
- The client crashes when processing `ClientboundSetEntityDataPacket` for that Wolf

### Notes
- Server logs show zero errors — the crash is purely client-side deserialization
- Breaking a sculk sensor and rejoining avoids the crash (likely moves player away from wolf)
- Completely unrelated to game event / vibration system changes
