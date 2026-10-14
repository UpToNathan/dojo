use std::io;
use std::process::{Child, ExitStatus};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

#[derive(Clone)]
pub(crate) struct SharedChild(Arc<ChildControl>);
struct ChildControl {
    child: Mutex<Child>,
    retired: AtomicBool,
}

impl SharedChild {
    pub(crate) fn new(child: Child) -> Self {
        Self(Arc::new(ChildControl {
            child: Mutex::new(child),
            retired: AtomicBool::new(false),
        }))
    }
    pub(crate) fn id(&self) -> u32 {
        self.0.child.lock().unwrap().id()
    }
    pub(crate) fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.0.child.lock().unwrap().try_wait()
    }
    pub(crate) fn kill(&mut self) -> io::Result<()> {
        let mut child = self.0.child.lock().unwrap();
        if child.try_wait()?.is_none() {
            child.kill()?;
        }
        Ok(())
    }
    pub(crate) fn wait(&mut self) -> io::Result<ExitStatus> {
        self.0.child.lock().unwrap().wait()
    }
    pub(crate) fn is_retired(&self) -> bool {
        self.0.retired.load(Ordering::Acquire)
    }
    pub(crate) fn retire(&self) -> io::Result<()> {
        self.0.retired.store(true, Ordering::Release);
        let mut child = self.0.child.lock().unwrap();
        if child.try_wait()?.is_none() {
            child.kill()?;
            child.wait()?;
        }
        Ok(())
    }
}
