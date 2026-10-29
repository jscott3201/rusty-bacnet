//! TestTransport send plans and explicit notification delivery barriers.
use super::*;

impl ClockLink {
    /// The plan's action if `apdu` is the COV-multiple notification it names.
    fn planned(&self, apdu: &Apdu) -> Option<PlanAction> {
        let mut plan = self.plan.lock().unwrap();
        let single = matches!(apdu, Apdu::UnconfirmedRequest(r) if r.service_choice == UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION)
            || matches!(apdu, Apdu::ConfirmedRequest(r) if r.service_choice == ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION);
        if !plan.as_ref().is_some_and(|p| {
            if p.single {
                single
            } else {
                is_cov_multiple(apdu)
            }
        }) {
            return None;
        }
        match plan.as_mut() {
            Some(pending) if pending.after > 0 => {
                pending.after -= 1;
                None
            }
            Some(_) => plan.take().map(|due| due.action),
            None => None,
        }
    }

    pub(super) async fn send(self, frame: SentFrame) -> Result<(), Error> {
        if frame.broadcast {
            if let Some(next) = self.after_broadcast.lock().unwrap().take() {
                *self.clock.0.lock().unwrap() = next;
            }
            return Ok(());
        }
        let apdu = frame.apdu();
        match &apdu {
            Apdu::SimpleAck(_) | Apdu::Error(_) => {
                if let Some(next) = self.after_ack.lock().unwrap().take() {
                    *self.clock.0.lock().unwrap() = next;
                }
            }
            Apdu::UnconfirmedRequest(_) | Apdu::ConfirmedRequest(_)
                if self.fail_notifications.load(Ordering::Acquire) =>
            {
                return Err(Error::Encoding("injected notification send failure".into()));
            }
            _ => {}
        }
        match self.planned(&apdu) {
            Some(PlanAction::Hold(release)) => {
                release.acquire().await.unwrap().forget();
                self.frames.lock().unwrap().push(apdu);
            }
            Some(PlanAction::BarrierHold { entered, release }) => {
                entered.add_permits(1);
                release.acquire().await.unwrap().forget();
                self.frames.lock().unwrap().push(apdu);
            }
            Some(PlanAction::Fail) => {
                return Err(Error::Encoding("injected notification send failure".into()));
            }
            Some(PlanAction::Disable(comm_state)) => {
                self.frames.lock().unwrap().push(apdu);
                comm_state.set_for_test(DccState::DisableInitiation);
            }
            None => self.frames.lock().unwrap().push(apdu),
        }
        Ok(())
    }
}
