use proc_macro::TokenStream;
use quote::quote;
use syn::{
    parse_macro_input, Data, DeriveInput, Expr, Field, Fields, GenericArgument, Ident,
    PathArguments, Type, TypeParamBound,
};

#[proc_macro_derive(Builder, attributes(specify_later, default))]
pub fn derive_builder(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    // Extract struct name and fields
    let struct_name = &input.ident;
    let fields = match extract_fields(&input) {
        Ok(fields) => fields,
        Err(err) => return err.to_compile_error().into(),
    };

    // Create builder name
    let builder_name = create_builder_name(struct_name);

    // Generate all components
    let builder_struct = match generate_builder_struct(input.vis, &builder_name, &fields) {
        Ok(s) => s,
        Err(err) => return err.to_compile_error().into(),
    };
    let type_aliases = match generate_type_aliases(struct_name, &builder_name, &fields) {
        Ok(s) => s,
        Err(err) => return err.to_compile_error().into(),
    };
    let constructor = match generate_constructor(&builder_name, &fields) {
        Ok(s) => s,
        Err(err) => return err.to_compile_error().into(),
    };
    let builder_methods = match generate_builder_methods(&builder_name, &fields) {
        Ok(s) => s,
        Err(err) => return err.to_compile_error().into(),
    };
    let build_method = match generate_build_method(struct_name, &builder_name, &fields) {
        Ok(s) => s,
        Err(err) => return err.to_compile_error().into(),
    };

    let expanded = quote! {
        #builder_struct
        #type_aliases
        #constructor
        #builder_methods
        #build_method
    };

    TokenStream::from(expanded)
}

/// Extract named fields from struct
fn extract_fields(input: &DeriveInput) -> syn::Result<Vec<Field>> {
    match &input.data {
        Data::Struct(data_struct) => match &data_struct.fields {
            Fields::Named(fields) => Ok(fields.named.iter().cloned().collect()),
            _ => Err(syn::Error::new_spanned(
                &data_struct.fields,
                "Builder can only be derived for structs with named fields",
            )),
        },
        _ => Err(syn::Error::new_spanned(
            input,
            "Builder can only be derived for structs",
        )),
    }
}

/// Check if a field has the #[specify_later] attribute
fn has_specify_later_attr(field: &Field) -> bool {
    field
        .attrs
        .iter()
        .any(|attr| attr.path().is_ident("specify_later"))
}

/// Extract default value from #[default(...)] attribute
fn extract_default_value(field: &Field) -> Option<Expr> {
    for attr in &field.attrs {
        if attr.path().is_ident("default") {
            if let Ok(expr) = attr.parse_args::<Expr>() {
                return Some(expr);
            }
        }
    }
    None
}

/// Check if a field has a #[default(...)] attribute
fn has_default_attr(field: &Field) -> bool {
    extract_default_value(field).is_some()
}

/// Check if any field has the #[specify_later] attribute (excluding default fields)
fn has_any_specify_later_fields(fields: &[Field]) -> bool {
    fields.iter().any(|f| {
        // Only count as specify_later if it has #[specify_later] but NOT #[default]
        has_specify_later_attr(f) && !has_default_attr(f)
    })
}

// ============================================================================
// Field Categorization
// ============================================================================

/// Categories for fields based on their attributes
#[allow(clippy::large_enum_variant)]
enum FieldCategory {
    Default {
        value: Expr, // The default value expression
        ty: Type,    // The concrete type
    },
    Regular {
        ty: Type, // The type (may need smart wrapping)
    },
}

/// A categorized field with its name, category, and wrapper type
struct CategorizedField {
    name: Ident,
    category: FieldCategory,
    wrapper: WrapperType,
    has_specify_later: bool, // Track if field has #[specify_later] attribute
}

/// Categorize a field as either default or regular
fn categorize_field(field: &Field) -> syn::Result<CategorizedField> {
    let name = field.ident.as_ref().unwrap().clone();
    let ty = field.ty.clone();
    let wrapper = analyze_field_type(&ty);
    let has_specify_later = has_specify_later_attr(field);
    let default_value = extract_default_value(field);

    // #[default] and #[specify_later] are mutually exclusive
    if default_value.is_some() && has_specify_later {
        return Err(syn::Error::new_spanned(
            field,
            "field cannot have both #[default] and #[specify_later] attributes - they are mutually exclusive"
        ));
    }

    // If explicit default is provided, use it
    if let Some(default_value) = default_value {
        return Ok(CategorizedField {
            name,
            category: FieldCategory::Default {
                value: default_value,
                ty,
            },
            wrapper,
            has_specify_later: false,
        });
    }

    if matches!(
        wrapper,
        WrapperType::Option(_) | WrapperType::OptionArc(_) | WrapperType::OptionBox(_)
    ) {
        return Ok(CategorizedField {
            name,
            category: FieldCategory::Default {
                value: syn::parse_quote!(None),
                ty,
            },
            wrapper,
            has_specify_later: false,
        });
    }

    // Regular field
    Ok(CategorizedField {
        name,
        category: FieldCategory::Regular { ty },
        wrapper,
        has_specify_later,
    })
}

/// Categorize all fields
fn categorize_fields(fields: &[Field]) -> syn::Result<Vec<CategorizedField>> {
    fields.iter().map(categorize_field).collect()
}

/// Convert field name to PascalCase type parameter
/// Converts snake_case like "api_key" to PascalCase like "ApiKey"
fn field_name_to_type_param(name: &Ident) -> Ident {
    let name_str = name.to_string();
    let pascal_case = name_str
        .split('_')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            }
        })
        .collect::<String>();
    Ident::new(&pascal_case, name.span())
}

/// Create builder name (e.g., "Config" -> "ConfigBuilder")
fn create_builder_name(struct_name: &Ident) -> Ident {
    let name = format!("{struct_name}Builder");
    Ident::new(&name, struct_name.span())
}

/// Create type alias name (e.g., "Config" + "Incomplete" -> "ConfigIncomplete")
fn create_type_alias_name(struct_name: &Ident, suffix: &str) -> Ident {
    let name = format!("{struct_name}{suffix}");
    Ident::new(&name, struct_name.span())
}

// ============================================================================
// Type Analysis for Smart Wrappers
// ============================================================================

/// Represents different wrapper types for fields
enum WrapperType {
    None,                           // Regular field, no wrapping
    Arc(Vec<TypeParamBound>),       // Arc<dyn Trait> - bounds are the trait bounds
    Box(Vec<TypeParamBound>),       // Box<dyn Trait> - bounds are the trait bounds
    Option(Box<Type>),              // Option<T> - store inner type
    OptionArc(Vec<TypeParamBound>), // Option<Arc<dyn Trait>>
    OptionBox(Vec<TypeParamBound>), // Option<Box<dyn Trait>>
}

/// Check if a type is Option<T> and extract the inner type
fn is_option_type(ty: &Type) -> Option<Type> {
    if let Type::Path(type_path) = ty {
        let last_segment = type_path.path.segments.last()?;

        if last_segment.ident != "Option" {
            return None;
        }

        if let PathArguments::AngleBracketed(args) = &last_segment.arguments {
            if let Some(GenericArgument::Type(inner_ty)) = args.args.first() {
                return Some(inner_ty.clone());
            }
        }
    }

    None
}

/// Check if a type is Arc<dyn Trait> and extract the trait bounds
fn is_arc_dyn_trait(ty: &Type) -> Option<Vec<TypeParamBound>> {
    if let Type::Path(type_path) = ty {
        // Check if the path has segments and the last one is "Arc"
        let last_segment = type_path.path.segments.last()?;

        if last_segment.ident != "Arc" {
            return None;
        }

        // Get generic arguments
        if let PathArguments::AngleBracketed(args) = &last_segment.arguments {
            if let Some(GenericArgument::Type(Type::TraitObject(trait_obj))) = args.args.first() {
                return Some(trait_obj.bounds.iter().cloned().collect());
            }
        }
    }

    None
}

/// Check if a type is Box<dyn Trait> and extract the trait bounds
fn is_box_dyn_trait(ty: &Type) -> Option<Vec<TypeParamBound>> {
    if let Type::Path(type_path) = ty {
        // Check if the path has segments and the last one is "Box"
        let last_segment = type_path.path.segments.last()?;

        if last_segment.ident != "Box" {
            return None;
        }

        // Get generic arguments
        if let PathArguments::AngleBracketed(args) = &last_segment.arguments {
            if let Some(GenericArgument::Type(Type::TraitObject(trait_obj))) = args.args.first() {
                return Some(trait_obj.bounds.iter().cloned().collect());
            }
        }
    }

    None
}

/// Analyze a field type to determine if it needs smart wrapping
fn analyze_field_type(ty: &Type) -> WrapperType {
    // Check for Option<Arc<dyn Trait>> or Option<Box<dyn Trait>>
    if let Some(inner_ty) = is_option_type(ty) {
        if let Some(bounds) = is_arc_dyn_trait(&inner_ty) {
            return WrapperType::OptionArc(bounds);
        }
        if let Some(bounds) = is_box_dyn_trait(&inner_ty) {
            return WrapperType::OptionBox(bounds);
        }
        return WrapperType::Option(Box::new(inner_ty));
    }

    if let Some(bounds) = is_arc_dyn_trait(ty) {
        return WrapperType::Arc(bounds);
    }

    if let Some(bounds) = is_box_dyn_trait(ty) {
        return WrapperType::Box(bounds);
    }

    WrapperType::None
}

// ============================================================================
// Code Generation Functions
// ============================================================================

/// Generate the builder struct definition
fn generate_builder_struct(
    vis: syn::Visibility,
    builder_name: &Ident,
    fields: &[Field],
) -> syn::Result<proc_macro2::TokenStream> {
    let categorized = categorize_fields(fields)?;

    // Only non-default fields get generic parameters
    let type_params: Vec<_> = categorized
        .iter()
        .filter_map(|f| match &f.category {
            FieldCategory::Regular { .. } => Some(field_name_to_type_param(&f.name)),
            FieldCategory::Default { .. } => None,
        })
        .collect();

    let field_defs: Vec<_> = categorized
        .iter()
        .map(|f| {
            let field_name = &f.name;
            match &f.category {
                FieldCategory::Default { ty, .. } => {
                    // Default fields have concrete types
                    quote! { pub #field_name: #ty }
                }
                FieldCategory::Regular { .. } => {
                    // Regular fields are generic
                    let type_param = field_name_to_type_param(field_name);
                    quote! { pub #field_name: #type_param }
                }
            }
        })
        .collect();

    Ok(quote! {
        #[derive(Clone, Debug)]
        #vis struct #builder_name<#(#type_params),*> {
            #(#field_defs),*
        }
    })
}

/// Generate type aliases (conditionally Incomplete only)
fn generate_type_aliases(
    struct_name: &Ident,
    builder_name: &Ident,
    fields: &[Field],
) -> syn::Result<proc_macro2::TokenStream> {
    // Only generate specify_later if there are #[specify_later] markers
    if has_any_specify_later_fields(fields) {
        let categorized = categorize_fields(fields)?;

        // Only include regular fields in type parameters (default fields are concrete)
        let type_params_specify_later: Vec<_> = categorized
            .iter()
            .filter_map(|f| match &f.category {
                FieldCategory::Regular { ty } => {
                    if f.has_specify_later {
                        Some(quote! { () })
                    } else {
                        Some(quote! { #ty })
                    }
                }
                FieldCategory::Default { .. } => None,
            })
            .collect();

        let incomplete_name = create_type_alias_name(struct_name, "Incomplete");
        Ok(quote! {
            pub type #incomplete_name = #builder_name<#(#type_params_specify_later),*>;
        })
    } else {
        Ok(quote! {})
    }
}

/// Generate the new() constructor
fn generate_constructor(
    builder_name: &Ident,
    fields: &[Field],
) -> syn::Result<proc_macro2::TokenStream> {
    let categorized = categorize_fields(fields)?;

    // Only regular fields get () type parameters
    let type_params: Vec<_> = categorized
        .iter()
        .filter_map(|f| match &f.category {
            FieldCategory::Regular { .. } => Some(quote! { () }),
            FieldCategory::Default { .. } => None,
        })
        .collect();

    let field_inits: Vec<_> = categorized
        .iter()
        .map(|f| {
            let field_name = &f.name;
            match &f.category {
                FieldCategory::Default { value, .. } => {
                    // Initialize with default value
                    quote! { #field_name: #value }
                }
                FieldCategory::Regular { .. } => {
                    // Initialize with ()
                    quote! { #field_name: () }
                }
            }
        })
        .collect();

    Ok(quote! {
        impl #builder_name<#(#type_params),*> {
            pub fn new() -> Self {
                Self {
                    #(#field_inits),*
                }
            }
        }
    })
}

/// Generate builder methods for each field
fn generate_builder_methods(
    builder_name: &Ident,
    fields: &[Field],
) -> syn::Result<proc_macro2::TokenStream> {
    let categorized = categorize_fields(fields)?;

    // Generate type parameters (only for regular fields)
    let type_params: Vec<_> = categorized
        .iter()
        .filter_map(|f| match &f.category {
            FieldCategory::Regular { .. } => Some(field_name_to_type_param(&f.name)),
            FieldCategory::Default { .. } => None,
        })
        .collect();

    // Split categorized fields into regular and default
    let regular_fields: Vec<_> = categorized
        .iter()
        .enumerate()
        .filter_map(|(idx, f)| match &f.category {
            FieldCategory::Regular { .. } => Some((idx, f)),
            FieldCategory::Default { .. } => None,
        })
        .collect();

    let default_fields: Vec<_> = categorized
        .iter()
        .filter(|f| matches!(&f.category, FieldCategory::Default { .. }))
        .collect();

    // Generate methods for regular fields (typestate pattern)
    let regular_methods: Vec<_> = regular_fields
        .iter()
        .enumerate()
        .map(|(regular_idx, (_, catfield))| {
            let field_name = &catfield.name;
            let field_type = match &catfield.category {
                FieldCategory::Regular { ty } => ty,
                _ => unreachable!(),
            };
            let method_name = Ident::new(&format!("with_{field_name}"), field_name.span());

            // Create return type parameters (replace the current field's generic with concrete type)
            let return_type_params: Vec<_> = type_params
                .iter()
                .enumerate()
                .map(|(i, param)| {
                    if i == regular_idx {
                        quote! { #field_type }
                    } else {
                        quote! { #param }
                    }
                })
                .collect();

            // Create field assignments for all other fields (regular and default)
            let other_field_assignments: Vec<_> = categorized
                .iter()
                .filter(|f| f.name != *field_name)
                .map(|f| {
                    let fname = &f.name;
                    quote! { #fname: self.#fname }
                })
                .collect();

            match &catfield.wrapper {
                WrapperType::Arc(bounds) => {
                    let has_lifetime = bounds.iter().any(|b| matches!(b, TypeParamBound::Lifetime(_)));
                    let trait_bounds = if has_lifetime {
                        quote! { #(#bounds)+* }
                    } else {
                        quote! { #(#bounds)+* + 'static }
                    };

                    quote! {
                        pub fn #method_name(
                            self,
                            #field_name: impl #trait_bounds
                        ) -> #builder_name<#(#return_type_params),*> {
                            #builder_name {
                                #field_name: std::sync::Arc::new(#field_name),
                                #(#other_field_assignments),*
                            }
                        }
                    }
                }

                WrapperType::Box(bounds) => {
                    let has_lifetime = bounds.iter().any(|b| matches!(b, TypeParamBound::Lifetime(_)));
                    let trait_bounds = if has_lifetime {
                        quote! { #(#bounds)+* }
                    } else {
                        quote! { #(#bounds)+* + 'static }
                    };

                    quote! {
                        pub fn #method_name(
                            self,
                            #field_name: impl #trait_bounds
                        ) -> #builder_name<#(#return_type_params),*> {
                            #builder_name {
                                #field_name: Box::new(#field_name),
                                #(#other_field_assignments),*
                            }
                        }
                    }
                }

                WrapperType::None => {
                    quote! {
                        pub fn #method_name(self, #field_name: impl Into<#field_type>) -> #builder_name<#(#return_type_params),*> {
                            #builder_name {
                                #field_name: #field_name.into(),
                                #(#other_field_assignments),*
                            }
                        }
                    }
                }

                // Option types should never be regular fields, only default
                WrapperType::Option(_) | WrapperType::OptionArc(_) | WrapperType::OptionBox(_) => {
                    unreachable!("Option fields should always be categorized as Default")
                }
            }
        })
        .collect();

    // Generate mutable consuming setters for default fields
    let default_methods: Vec<_> = default_fields
        .iter()
        .map(|catfield| {
            let field_name = &catfield.name;
            let field_type = match &catfield.category {
                FieldCategory::Default { ty, .. } => ty,
                _ => unreachable!(),
            };
            let method_name = Ident::new(&format!("with_{field_name}"), field_name.span());

            match &catfield.wrapper {
                WrapperType::Arc(bounds) => {
                    let has_lifetime = bounds
                        .iter()
                        .any(|b| matches!(b, TypeParamBound::Lifetime(_)));
                    let trait_bounds = if has_lifetime {
                        quote! { #(#bounds)+* }
                    } else {
                        quote! { #(#bounds)+* + 'static }
                    };

                    quote! {
                        pub fn #method_name(
                            mut self,
                            #field_name: impl #trait_bounds
                        ) -> Self {
                            self.#field_name = std::sync::Arc::new(#field_name);
                            self
                        }
                    }
                }

                WrapperType::Box(bounds) => {
                    let has_lifetime = bounds
                        .iter()
                        .any(|b| matches!(b, TypeParamBound::Lifetime(_)));
                    let trait_bounds = if has_lifetime {
                        quote! { #(#bounds)+* }
                    } else {
                        quote! { #(#bounds)+* + 'static }
                    };

                    quote! {
                        pub fn #method_name(
                            mut self,
                            #field_name: impl #trait_bounds
                        ) -> Self {
                            self.#field_name = Box::new(#field_name);
                            self
                        }
                    }
                }

                WrapperType::Option(inner_ty) => {
                    quote! {
                        pub fn #method_name(mut self, #field_name: impl Into<Option<#inner_ty>>) -> Self {
                            self.#field_name = #field_name.into();
                            self
                        }
                    }
                }

                WrapperType::OptionArc(bounds) => {
                    let has_lifetime = bounds
                        .iter()
                        .any(|b| matches!(b, TypeParamBound::Lifetime(_)));
                    let trait_bounds = if has_lifetime {
                        quote! { #(#bounds)+* }
                    } else {
                        quote! { #(#bounds)+* + 'static }
                    };

                    quote! {
                        pub fn #method_name(
                            mut self,
                            #field_name: impl Into<Option<impl #trait_bounds>>
                        ) -> Self {
                            self.#field_name = #field_name.into().map(|v| std::sync::Arc::new(v));
                            self
                        }
                    }
                }

                WrapperType::OptionBox(bounds) => {
                    let has_lifetime = bounds
                        .iter()
                        .any(|b| matches!(b, TypeParamBound::Lifetime(_)));
                    let trait_bounds = if has_lifetime {
                        quote! { #(#bounds)+* }
                    } else {
                        quote! { #(#bounds)+* + 'static }
                    };

                    quote! {
                        pub fn #method_name(
                            mut self,
                            #field_name: impl Into<Option<impl #trait_bounds>>
                        ) -> Self {
                            self.#field_name = #field_name.into().map(|v| Box::new(v));
                            self
                        }
                    }
                }

                WrapperType::None => {
                    quote! {
                        pub fn #method_name(mut self, #field_name: impl Into<#field_type>) -> Self {
                            self.#field_name = #field_name.into();
                            self
                        }
                    }
                }
            }
        })
        .collect();

    Ok(quote! {
        impl<#(#type_params),*> #builder_name<#(#type_params),*> {
            #(#regular_methods)*
            #(#default_methods)*
        }
    })
}

/// Generate build() method for fully concrete builder
fn generate_build_method(
    struct_name: &Ident,
    builder_name: &Ident,
    fields: &[Field],
) -> syn::Result<proc_macro2::TokenStream> {
    let categorized = categorize_fields(fields)?;

    // Only regular fields are type parameters (default fields are already concrete)
    let concrete_types: Vec<_> = categorized
        .iter()
        .filter_map(|f| match &f.category {
            FieldCategory::Regular { ty } => Some(ty),
            FieldCategory::Default { .. } => None,
        })
        .collect();

    // All fields are included in the final struct
    let field_names: Vec<_> = categorized.iter().map(|f| &f.name).collect();

    Ok(quote! {
        impl #builder_name<#(#concrete_types),*> {
            pub fn build(self) -> #struct_name {
                #struct_name {
                    #(#field_names: self.#field_names),*
                }
            }
        }
    })
}
