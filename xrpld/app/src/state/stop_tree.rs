//! Explicit shutdown tree replacement for the stale reference `Stoppable` graph.
//!
//! The current Rust port keeps shutdown semantics honest by storing callbacks
//! explicitly and walking them in a deterministic post-order.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct StopTreeNode {
    name: String,
    callback: Option<Arc<dyn Fn() + Send + Sync>>,
    children: Arc<Mutex<Vec<Arc<StopTreeNode>>>>,
}

impl std::fmt::Debug for StopTreeNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StopTreeNode")
            .field("name", &self.name)
            .field("children", &self.child_count())
            .finish()
    }
}

impl StopTreeNode {
    fn new(name: impl Into<String>, callback: Option<Arc<dyn Fn() + Send + Sync>>) -> Arc<Self> {
        Arc::new(Self {
            name: name.into(),
            callback,
            children: Arc::new(Mutex::new(Vec::new())),
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn child_count(&self) -> usize {
        self.children
            .lock()
            .expect("stop-tree children mutex must not be poisoned")
            .len()
    }

    fn add_child(
        parent: &Arc<Self>,
        name: impl Into<String>,
        callback: Option<Arc<dyn Fn() + Send + Sync>>,
    ) -> Arc<Self> {
        let child = Self::new(name, callback);
        parent
            .children
            .lock()
            .expect("stop-tree children mutex must not be poisoned")
            .push(Arc::clone(&child));
        child
    }

    fn stop_post_order(node: &Arc<Self>) {
        let children = node
            .children
            .lock()
            .expect("stop-tree children mutex must not be poisoned")
            .clone();

        for child in children.into_iter().rev() {
            Self::stop_post_order(&child);
        }

        if let Some(callback) = &node.callback {
            callback();
        }
    }
}

#[derive(Clone)]
pub struct StopTree {
    root: Arc<StopTreeNode>,
    stopping: Arc<AtomicBool>,
    reason: Arc<Mutex<Option<String>>>,
}

impl std::fmt::Debug for StopTree {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StopTree")
            .field("stopping", &self.is_stopping())
            .field("reason", &self.reason())
            .finish()
    }
}

impl StopTree {
    pub fn new(root_name: impl Into<String>) -> Self {
        Self {
            root: StopTreeNode::new(root_name, None),
            stopping: Arc::new(AtomicBool::new(false)),
            reason: Arc::new(Mutex::new(None)),
        }
    }

    pub fn root(&self) -> Arc<StopTreeNode> {
        Arc::clone(&self.root)
    }

    pub fn register_callback(
        &self,
        name: impl Into<String>,
        callback: impl Fn() + Send + Sync + 'static,
    ) -> Arc<StopTreeNode> {
        StopTreeNode::add_child(&self.root, name, Some(Arc::new(callback)))
    }

    pub fn register_child(
        &self,
        parent: &Arc<StopTreeNode>,
        name: impl Into<String>,
        callback: impl Fn() + Send + Sync + 'static,
    ) -> Arc<StopTreeNode> {
        StopTreeNode::add_child(parent, name, Some(Arc::new(callback)))
    }

    pub fn is_stopping(&self) -> bool {
        self.stopping.load(Ordering::Acquire)
    }

    pub fn reason(&self) -> Option<String> {
        self.reason
            .lock()
            .expect("stop-tree reason mutex must not be poisoned")
            .clone()
    }

    pub fn signal_stop(&self, reason: impl Into<String>) -> bool {
        if self.stopping.swap(true, Ordering::AcqRel) {
            return false;
        }

        *self
            .reason
            .lock()
            .expect("stop-tree reason mutex must not be poisoned") = Some(reason.into());
        StopTreeNode::stop_post_order(&self.root);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::StopTree;
    use std::sync::{Arc, Mutex};

    #[test]
    fn stop_tree_runs_children_before_parent_once() {
        let tree = StopTree::new("app");
        let events = Arc::new(Mutex::new(Vec::new()));

        let parent_events = Arc::clone(&events);
        let parent = tree.register_callback("parent", move || {
            parent_events
                .lock()
                .expect("events mutex must not be poisoned")
                .push("parent");
        });

        let child_events = Arc::clone(&events);
        tree.register_child(&parent, "child", move || {
            child_events
                .lock()
                .expect("events mutex must not be poisoned")
                .push("child");
        });

        assert!(tree.signal_stop("finished"));
        assert!(!tree.signal_stop("ignored"));
        assert_eq!(tree.reason(), Some("finished".to_owned()));
        assert_eq!(
            events
                .lock()
                .expect("events mutex must not be poisoned")
                .as_slice(),
            &["child", "parent"]
        );
    }
}
