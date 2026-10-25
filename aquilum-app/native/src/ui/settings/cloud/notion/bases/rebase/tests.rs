//! Native Masonry pointer event proof for recovery buttons after restart.
//! Does not simulate a privileged OS file-write consent dialog.
use super::*;
use aquilum_core::notion_api_probe::bases_edits::{RestoreArchive,RestoreCell};
use masonry::app::{RenderRoot,RenderRootOptions,RenderRootSignal,WindowSizePolicy};
use masonry::core::{
    PointerButton,PointerButtonEvent,PointerEvent,PointerId,
    PointerInfo,PointerState,PointerType,PointerUpdate,WidgetId,
};
use masonry::dpi::{PhysicalPosition,PhysicalSize};
use crate::ui::theme;
use std::sync::{Arc,Mutex};

fn mouse()->PointerInfo {
    PointerInfo {
        pointer_id:Some(PointerId::PRIMARY),
        persistent_device_id:None,pointer_type:PointerType::Mouse,
    }
}
fn click(root:&mut RenderRoot,id:WidgetId,signals:&Arc<Mutex<Vec<WidgetId>>>) {
    let rect=root.get_widget(id).expect("mounted archive action").ctx().bounding_box();
    assert!(rect.width()>0.0 && rect.height()>0.0, "archive button has no layout");
    let position=PhysicalPosition::new(rect.center().x,rect.center().y);
    let mut state=PointerState {
        position,count:1,scale_factor:1.0,..PointerState::default()
    };
    root.handle_pointer_event(PointerEvent::Move(PointerUpdate {
        pointer:mouse(),current:state.clone(),coalesced:vec![],predicted:vec![],
    }));
    state.buttons.insert(PointerButton::Primary);
    root.handle_pointer_event(PointerEvent::Down(PointerButtonEvent {
        pointer:mouse(),button:Some(PointerButton::Primary),state:state.clone(),
    }));
    state.buttons.remove(PointerButton::Primary);
    root.handle_pointer_event(PointerEvent::Up(PointerButtonEvent {
        pointer:mouse(),button:Some(PointerButton::Primary),state,
    }));
    let _=root.redraw();
    assert!(signals.lock().unwrap().contains(&id),
        "real pointer events did not activate the exact archive action");
    signals.lock().unwrap().clear();
}
#[test]
fn real_native_pointer_dispatches_scan_and_exact_archived_cell_identity() {
    let _guard=crate::ui::theme::test_lock();
    let archived="Notion-import/root-abc.akula-bases-backup.json";
    let row="row-id-1";
    let prop="property-id-2";
    let mut view=BasesView {
        base_id:Some("base".into()),
        digest:Some("a".repeat(64)),
        ..Default::default()
    };
    view.archives.push(RestoreArchive {
        path:archived.into(),
        old_digest:"b".repeat(64),
        new_digest:"a".repeat(64),
        archived_hash:"c".repeat(64),
        cells:vec![RestoreCell {
            row_id:row.into(),property_id:prop.into(),
            value_preview:"Прежнее значение Akula".into(),
        }],
    });
    let mut bindings=HashMap::new();
    let widget=NewWidget::new(append(Flex::column(),&view,&mut bindings));
    let scan=bindings.iter().find_map(|(id,b)|matches!(
        b,Binding::Button(Action::NotionBasesScanArchives)
    ).then_some(*id)).expect("scan archive action mounted");
    let restore=bindings.iter().find_map(|(id,b)|matches!(
        b,Binding::Button(Action::NotionBasesRestoreListed(path,r,p))
            if path==archived && r==row && p==prop
    ).then_some(*id)).expect("exact restore action mounted");
    assert_ne!(scan,restore,"scan and restore must be independent actions");
    let signals=Arc::new(Mutex::new(Vec::<WidgetId>::new()));
    let sink=Arc::clone(&signals);
    let options=RenderRootOptions {
        default_properties:Arc::new(theme::default_properties()),
        use_system_fonts:true,
        size_policy:WindowSizePolicy::User,
        size:PhysicalSize::new(1700,1100),
        scale_factor:1.0,test_font:None,
    };
    let mut root=RenderRoot::new(widget,move |signal|{
        if let RenderRootSignal::Action(_,id)=signal {
            sink.lock().unwrap().push(id);
        }
    },options);
    theme::register_fonts(&mut root);
    let _=root.redraw();
    click(&mut root,scan,&signals);
    click(&mut root,restore,&signals);
}
#[test]
fn archive_restore_action_is_absent_without_verified_candidates() {
    let view=BasesView {
        base_id:Some("base".into()),
        digest:Some("a".repeat(64)),
        ..Default::default()
    };
    let mut bindings=HashMap::new();
    let _=append(Flex::column(),&view,&mut bindings);
    assert!(bindings.values().any(|b|matches!(
        b,Binding::Button(Action::NotionBasesScanArchives)
    )));
    assert!(!bindings.values().any(|b|matches!(
        b,Binding::Button(Action::NotionBasesRestoreListed(_,_,_))
    )));
}
