# AI Subscription Usage Monitor

This context describes a monitor for Claude and Codex subscription usage, usage-window renewals, and Codex banked manual resets.

## Language

**Provider**:
A subscription service whose allowance is monitored, currently Claude or Codex. Each provider has its own usage windows and allowance.
_Avoid_: combined quota

**Provider dashboard**:
A view of one provider's usage windows, renewal times, and available account information.
_Avoid_: overview

**Overview**:
A view that shows Claude and Codex usage side by side while keeping their allowances separate.
_Avoid_: combined usage percentage

**Usage window**:
A plan-defined allowance over a fixed period, such as five hours or one week. A five-hour window is not present on every plan.
_Avoid_: reset (when referring to a quota window)

**Window reset time**:
The time when a usage window renews its allowance.
_Avoid_: banked reset expiry, manual reset

**Banked reset**:
An available credit that permits a manual reset of Codex usage. Expiry details may be known, unavailable, or incomplete.
_Avoid_: window reset, reset countdown

**Manual reset**:
The user-initiated operation that applies an available banked reset through the Codex website.
_Avoid_: window reset

**Banked reset expiry**:
The deadline for using one banked reset.
_Avoid_: window reset time

**Unavailable usage**:
A current usage reading that cannot be retrieved. It is distinct from zero usage.
_Avoid_: 0% usage, offline usage

**Remaining usage**:
The unused percentage of a provider-defined allowance, derived only from a known usage reading.
_Avoid_: remaining tokens, combined quota

**Last-known usage**:
The most recent successfully retrieved usage reading. It remains last-known data after a retrieval failure or while awaiting a provider refresh.
_Avoid_: fresh usage, zero usage

**Device**:
The ESP32 touchscreen that displays provider usage.
_Avoid_: board, monitor

**Device setup**:
Giving the device the Wi-Fi network and broker details it needs to receive usage. It is done on the device through a temporary access point, and can be started again from the device.
_Avoid_: provisioning, settings, Wi-Fi setup

**Server settings**:
Operator-adjustable values that govern how the server retrieves and publishes usage, such as each provider's refresh interval.
_Avoid_: config, device setup

**Admin app**:
The web application where the operator reads Server settings, usage, and the values needed for Device setup.
_Avoid_: dashboard (a dashboard is a view of usage)

**Refresh interval**:
The time the server waits between successful usage retrievals for one provider. A provider rate limit can lengthen the wait but never shorten it.
_Avoid_: poll rate, backoff

**Display layout**:
One of the selectable arrangements in which the device shows usage. The operator chooses it in the Admin app.
_Avoid_: theme, screen
