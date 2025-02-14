pub mod behaviour_select;
pub mod connection_handler_select;

pub mod utils {

    /// Less boilerplate for simple handler functions that use callback to return some data.
    ///
    /// Example:
    /// ```ignore
    /// generate_handler_method_blocking!({
    ///     EventVariant1:method_name(parameter_name:ParameterType,..)->ReturnType;
    ///     EventVariant2:method_name(*param:ParamType)->ReturnType;
    ///     ..
    /// })
    /// ```
    ///
    /// Supplied parameters must be in the same type and order with EventVariant.
    /// `&self` is automatically filled in, but EventVariant must be a tuple.   
    /// Variant can hold no data(except callback), which means leaving the function parameter blank.  
    #[macro_export]
    macro_rules! generate_handler_method_blocking {
    {$($(#[$metas:meta])*$variant:ident:$name:ident($($params:ident:$param_type:ty)*)->$return_type:ty;)+} => {
        $(
            $(#[$metas])*
            pub fn $name(&self,$($params:$param_type)*)->$return_type{
                use tokio::sync::oneshot::*;
                let (tx,rx) = channel();
                let ev = InEvent::$variant($($params,)*tx);
                self.sender.blocking_send(ev).expect(owlnest_core::expect::SWARM_RECEIVER_KEPT_ALIVE);
                rx.blocking_recv().expect(owlnest_core::expect::CALLBACK_CLEAR)
            }
        )*
    };
}

    /// Less boilerplate for simple handler functions that use callback to return some data.
    ///
    /// Example:
    /// ``` ignore
    /// generate_handler_method!({
    ///     EventVariant1:method_name(parameter_name:ParameterType,..)->ReturnType;
    ///     ..
    /// })
    /// ```
    ///
    /// Supplied parameters must be in the same type and order with EventVariant.
    /// `&self` is automatically filled in, but EventVariant must be a tuple.   
    /// Variant can hold no data(except callback), which means leaving the function parameter blank.  
    ///  
    /// Method with callback and without callback need to be generated separately.
    #[macro_export]
    macro_rules! generate_handler_method {
    {$($(#[$metas:meta])*$variant:ident:$name:ident($($params:ident:$param_type:ty$(,)?)*);)+} => {
        $(
            $(#[$metas])*
            pub async fn $name(&self,$($params:$param_type,)*){
                let ev = InEvent::$variant($($params,)*);
                self.sender.send(ev).await.expect(owlnest_core::expect::SWARM_RECEIVER_KEPT_ALIVE)
            }
        )*
    };

    {$($(#[$metas:meta])*$variant:ident:$name:ident($($params:ident:$param_type:ty$(,)?)*)->$return_type:ty;)+} => {
        $(
            $(#[$metas])*
            pub async fn $name(&self,$($params:$param_type,)*)->$return_type{
                use tokio::sync::oneshot::*;
                let (tx,rx) = channel();
                let ev = InEvent::$variant($($params,)*tx);
                self.sender.send(ev).await.expect(owlnest_core::expect::SWARM_RECEIVER_KEPT_ALIVE);
                rx.await.expect(owlnest_core::expect::CALLBACK_CLEAR)
            }
        )*
    };
    {$($(#[$metas:meta])*$variant:ident:$name:ident{$($params:ident:$param_type:ty$(,)?)*};)+} => {
        $(
            $(#[$metas])*
            pub async fn $name(&self,$($params:$param_type,)*){
                use tokio::sync::oneshot::*;
                let ev = InEvent::$variant{
                    $($params,)*
                };
                self.sender.send(ev).await.expect(owlnest_core::expect::SWARM_RECEIVER_KEPT_ALIVE);
            }
        )*
    };
    {$($(#[$metas:meta])*$variant:ident:$name:ident{$($params:ident:$param_type:ty$(,)?)*}->$return_type:ty;)+} => {
        $(
            $(#[$metas])*
            pub async fn $name(&self,$($params:$param_type,)*)->$return_type{
                use tokio::sync::oneshot::*;
                let (tx,rx) = channel();
                let ev = InEvent::$variant{
                    $($params,)*
                    callback: tx,
                };
                self.sender.send(ev).await.expect(owlnest_core::expect::SWARM_RECEIVER_KEPT_ALIVE);
                rx.await.expect(owlnest_core::expect::CALLBACK_CLEAR)
            }
        )*
   };
    {$($(#[$metas:meta])*$variant:ident:$name:ident({$($params:ident:$param_type:ty$(,)?)*});)+} => {
        $(
            $(#[$metas])*
            pub async fn $name(&self,$($params:$param_type,)*)->Result<(), crate::utils::ChannelError>{
                use tokio::sync::oneshot::*;
                let (tx,rx) = channel();
                let ev = InEvent::$variant{$($params,)*callback:tx};
                self.sender.send(ev).await.await.expect(owlnest_core::expect::SWARM_RECEIVER_KEPT_ALIVE);
                rx.await.expect(owlnest_core::expect::CALLBACK_CLEAR)
            }
        )*
    };
}

    #[macro_export]
    macro_rules! handle_callback_sender {
        // ($message:ident=>$sender:ident) => {
        //     if let Err(v) = $sender.send($message) {
        //         tracing::warn!("Cannot send a callback message: {:?}", v)
        //     }
        // };
        ($message:expr=>$sender:expr) => {
            if let Err(v) = $sender.send($message) {
                tracing::warn!("Cannot send a callback message: {:?}", v)
            }
        };
    }
    /// Short-hand for listening event on swarm.
    /// If `ops` does not contain explicit exit condition, it will listen on it forever.
    /// Best suited for one-shot event.
    #[macro_export]
    macro_rules! listen_event {
        ($listener:ident for $behaviour:ident,$($pattern:pat=>{$($ops:tt)+})+) => {
            async move{
                use crate::net::p2p::swarm::{SwarmEvent, BehaviourEvent};
                while let Ok(ev) = $listener.recv().await{
                    match ev.as_ref() {
                        $(SwarmEvent::Behaviour(BehaviourEvent::$behaviour($pattern)) => {$($ops)+})+
                        _ => {}
                    }
                }
                unreachable!()
            }
        };
    }
    #[macro_export]
    macro_rules! send_to_swam {
        ($ev:ident) => {
            self.sender.send(ev)
        };
    }
}
