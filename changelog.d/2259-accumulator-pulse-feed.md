---
section: Added
---
- An application can feed a registered Accumulator its input pulses with `BACnetServer::add_pulses_local`, a new `LocalInputValue::Pulses`: they step Present_Value through Prescale, and the server turns each Limit_Monitoring_Interval's count into Pulse_Rate (#2259).
