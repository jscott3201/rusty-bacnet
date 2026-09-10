---
title: "Observe changes with COV"
description: "Use finite subscriptions and understand what a notification does and does not prove."
---

Change of Value (COV) subscriptions can notify a client about selected property changes. This is an observation workflow, but it creates subscription state on the remote device. Obtain authorization and use a bounded lifetime.

## Watch one object

```sh
bacnet subscribe 192.168.1.100 ai:1 --lifetime 300
```

Use a known device and an object that supports the requested COV behavior. The lifetime is measured in seconds. The CLI watches for notifications; Ctrl+C stops the local watcher. Do not assume that stopping a client immediately removes remote state—verify cancellation behavior or allow the finite lifetime to expire.

## Read first, then subscribe

An initial property read establishes a baseline and gives useful context when no change notification arrives. Lack of notification is not proof that the network is broken or that the underlying value has remained exactly constant; investigate the object's COV behavior and increment as well as the notification delivery path.

## Python lifecycle

The Python client exposes `subscribe_cov`, `unsubscribe_cov`, and `cov_notifications()`. An application needs to coordinate subscribing, consuming notifications, renewing finite subscriptions where appropriate, and unsubscribing during orderly shutdown.

Use an explicit subscriber process identifier and keep the client alive while consuming events. Avoid copying a permanent subscription into a short-lived experiment. The API distinguishes confirmed and unconfirmed delivery; neither replaces application-level freshness checks or historian requirements.

## A working read is not a working notification path

A reply to a direct read and a later notification can exercise different address, lifetime, and firewall conditions. Recheck remote subscription state, the client's advertised/reachable endpoint, device support, and whether the process has restarted.

## Before using COV for telemetry

Define how your application represents missing data, detects stale values, handles reconnection, and re-establishes subscriptions. Qualify the selected device and network rather than assuming every implemented service works identically on every controller.

## Sources and release scope

These instructions target **v0.11.0**. Source review is not a claim of hardware qualification.

[CLI subscription command](https://github.com/jscott3201/rusty-bacnet/blob/v0.11.0/docs/CLI.md) · [Python subscription lifecycle](https://github.com/jscott3201/rusty-bacnet/blob/v0.11.0/docs/python-api.md) · [COV example](https://github.com/jscott3201/rusty-bacnet/blob/v0.11.0/examples/python/cov_subscriptions.py).
