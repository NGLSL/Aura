//! Overlay shell for searchable select: header in-flow, panel floats (no layout push).

use iced::advanced::layout::{self, Layout};
use iced::advanced::overlay;
use iced::advanced::renderer;
use iced::advanced::widget::Tree;
use iced::advanced::{Clipboard, Shell, Widget};
use iced::event::{self, Event};
use iced::mouse;
use iced::{Element, Length, Point, Rectangle, Size, Vector};

/// Header + optional floating panel under it. Panel never affects siblings.
pub struct DropdownOverlay<'a, Message, Theme, Renderer>
where
    Theme: 'a,
    Renderer: 'a,
{
    content: Element<'a, Message, Theme, Renderer>,
    panel: Option<Element<'a, Message, Theme, Renderer>>,
}

impl<'a, Message, Theme, Renderer> DropdownOverlay<'a, Message, Theme, Renderer>
where
    Theme: 'a,
    Renderer: 'a,
{
    pub fn new(
        content: impl Into<Element<'a, Message, Theme, Renderer>>,
        panel: Option<impl Into<Element<'a, Message, Theme, Renderer>>>,
    ) -> Self {
        Self {
            content: content.into(),
            panel: panel.map(Into::into),
        }
    }
}

impl<'a, Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for DropdownOverlay<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: 'a,
    Renderer: text::Renderer + 'a,
{
    fn children(&self) -> Vec<Tree> {
        let mut kids = vec![Tree::new(&self.content)];
        if let Some(p) = &self.panel {
            kids.push(Tree::new(p));
        }
        kids
    }

    fn diff(&self, tree: &mut Tree) {
        if self.panel.is_some() {
            tree.diff_children(&[
                self.content.as_widget(),
                self.panel.as_ref().unwrap().as_widget(),
            ]);
        } else {
            tree.diff_children(std::slice::from_ref(&self.content.as_widget()));
        }
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn size_hint(&self) -> Size<Length> {
        self.content.as_widget().size_hint()
    }

    fn layout(
        &self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content
            .as_widget()
            .layout(&mut tree.children[0], renderer, limits)
    }

    fn on_event(
        &mut self,
        tree: &mut Tree,
        event: Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) -> event::Status {
        self.content.as_widget_mut().on_event(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        )
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        let mut kids = tree.children.iter_mut();

        let content_overlay = self.content.as_widget_mut().overlay(
            kids.next().unwrap(),
            layout,
            renderer,
            translation,
        );

        let panel_overlay = self.panel.as_mut().and_then(|panel| {
            let state = kids.next()?;
            Some(overlay::Element::new(Box::new(PanelOverlay {
                panel,
                state,
                position: layout.position() + translation,
                content_bounds: layout.bounds(),
            })))
        });

        if content_overlay.is_some() || panel_overlay.is_some() {
            Some(
                overlay::Group::with_children(
                    content_overlay.into_iter().chain(panel_overlay).collect(),
                )
                .overlay(),
            )
        } else {
            None
        }
    }
}

impl<'a, Message, Theme, Renderer> From<DropdownOverlay<'a, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: 'a,
    Renderer: text::Renderer + 'a,
{
    fn from(w: DropdownOverlay<'a, Message, Theme, Renderer>) -> Self {
        Element::new(w)
    }
}

struct PanelOverlay<'a, 'b, Message, Theme, Renderer>
where
    Theme: 'a,
    Renderer: 'a,
{
    panel: &'b mut Element<'a, Message, Theme, Renderer>,
    state: &'b mut Tree,
    position: Point,
    content_bounds: Rectangle,
}

impl<'a, 'b, Message, Theme, Renderer> overlay::Overlay<Message, Theme, Renderer>
    for PanelOverlay<'a, 'b, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: 'a,
    Renderer: text::Renderer + 'a,
{
    fn layout(&mut self, renderer: &Renderer, _bounds: Size) -> layout::Node {
        // Panel width tracks the header field; height comes from the panel itself.
        let limits = layout::Limits::new(
            Size::new(self.content_bounds.width, 0.0),
            Size::new(self.content_bounds.width, 280.0),
        );
        let mut node = self.panel.as_widget().layout(self.state, renderer, &limits);
        // Force full field width so the list aligns with the combo header.
        let mut size = node.size();
        size.width = self.content_bounds.width;
        node = layout::Node::with_children(size, node.children().to_vec());
        node.move_to(Point::new(
            self.position.x,
            self.position.y + self.content_bounds.height + 2.0,
        ))
    }

    fn on_event(
        &mut self,
        event: Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
    ) -> event::Status {
        let bounds = layout.bounds();
        self.panel.as_widget_mut().on_event(
            self.state,
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            &bounds,
        )
    }

    fn mouse_interaction(
        &self,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.panel.as_widget().mouse_interaction(
            self.state,
            layout,
            cursor,
            viewport,
            renderer,
        )
    }

    fn draw(
        &self,
        renderer: &mut Renderer,
        theme: &Theme,
        defaults: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
    ) {
        let bounds = layout.bounds();
        self.panel.as_widget().draw(
            self.state,
            renderer,
            theme,
            defaults,
            layout,
            cursor,
            &bounds,
        );
    }

    fn is_over(
        &self,
        layout: Layout<'_>,
        _renderer: &Renderer,
        cursor_position: Point,
    ) -> bool {
        layout.bounds().contains(cursor_position)
    }
}

// Keep `text::Renderer` in scope via iced::advanced::text
use iced::advanced::text;
