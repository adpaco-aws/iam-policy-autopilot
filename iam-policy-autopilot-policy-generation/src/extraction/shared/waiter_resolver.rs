//! Shared waiter resolution utility for all language extractors
//!
//! This module provides a unified interface for resolving waiter names to AWS operations
//! across Python, Go, and JavaScript/TypeScript extractors. It eliminates the redundant
//! waiter resolution logic that was previously duplicated in each language extractor.

use crate::extraction::{Parameter, ParameterValue, SdkMethodCall, SdkMethodCallMetadata};
use crate::ServiceModelIndex;

/// Shared waiter resolver that handles waiter name → operation resolution
/// and synthetic call generation for all language extractors.
pub struct WaiterResolver<'a> {
    service_index: &'a ServiceModelIndex,
}

/// Information needed to create a synthetic waiter call
#[derive(Debug, Clone)]
pub struct WaiterCallInfo {
    /// The waiter name (e.g., "instance_terminated", "InstanceTerminated", "waitUntilBucketExists")
    pub waiter_name: String,
    /// Parameters extracted from the wait call (language-specific)
    pub parameters: Vec<Parameter>,
    /// Position information for the synthetic call
    pub start_position: (usize, usize),
    pub end_position: (usize, usize),
    /// Optional receiver information (e.g., client variable name)
    pub receiver: Option<String>,
    /// Services to filter to (if known from imports/context)
    pub possible_services: Option<Vec<String>>,
}

impl<'a> WaiterResolver<'a> {
    /// Create a new waiter resolver with the given service index
    pub fn new(service_index: &'a ServiceModelIndex) -> Self {
        Self { service_index }
    }

    /// Create synthetic SdkMethodCall objects for a waiter
    ///
    /// This method resolves the waiter name to underlying operations and creates
    /// synthetic method calls that represent the actual AWS operations being polled.
    ///
    /// # Arguments
    /// * `waiter_info` - Information about the waiter call to resolve
    ///
    /// # Returns
    /// A vector of synthetic SdkMethodCall objects, one per service that provides the waiter.
    pub fn create_synthetic_calls(&self, waiter_info: &WaiterCallInfo) -> Vec<SdkMethodCall> {
        let mut synthetic_calls = Vec::new();

        // Look up the waiter in the service index
        if let Some(service_methods) = self
            .service_index
            .waiter_lookup
            .get(&waiter_info.waiter_name)
        {
            for service_method in service_methods {
                // Filter by possible services if provided
                if let Some(ref possible_services) = waiter_info.possible_services {
                    if !possible_services.contains(&service_method.service_name) {
                        continue;
                    }
                }

                // Create synthetic call with the resolved operation name
                synthetic_calls.push(SdkMethodCall {
                    name: service_method.operation_name.clone(),
                    possible_services: vec![service_method.service_name.clone()],
                    metadata: Some(SdkMethodCallMetadata {
                        parameters: waiter_info.parameters.clone(),
                        return_type: None,
                        start_position: waiter_info.start_position,
                        end_position: waiter_info.end_position,
                        receiver: waiter_info.receiver.clone(),
                    }),
                });
            }
        }

        synthetic_calls
    }

    /// Create synthetic calls with required parameters for unmatched waiters
    ///
    /// When a waiter is found but no corresponding wait call is matched,
    /// this method creates synthetic calls with the required parameters
    /// for the underlying operation.
    ///
    /// # Arguments
    /// * `waiter_name` - The waiter name to resolve
    /// * `fallback_position` - Position to use for the synthetic calls
    /// * `receiver` - Optional receiver information
    /// * `possible_services` - Optional service filter
    ///
    /// # Returns
    /// A vector of synthetic SdkMethodCall objects with required parameters.
    pub fn create_fallback_synthetic_calls(
        &self,
        waiter_name: &str,
        fallback_position: (usize, usize),
        receiver: Option<String>,
        possible_services: Option<Vec<String>>,
    ) -> Vec<SdkMethodCall> {
        let mut synthetic_calls = Vec::new();

        if let Some(service_methods) = self.service_index.waiter_lookup.get(waiter_name) {
            for service_method in service_methods {
                // Filter by possible services if provided
                if let Some(ref possible_services) = possible_services {
                    if !possible_services.contains(&service_method.service_name) {
                        continue;
                    }
                }

                // Get required parameters for this operation
                let required_params = self.get_required_parameters(
                    &service_method.service_name,
                    &service_method.operation_name,
                );

                synthetic_calls.push(SdkMethodCall {
                    name: service_method.operation_name.clone(),
                    possible_services: vec![service_method.service_name.clone()],
                    metadata: Some(SdkMethodCallMetadata {
                        parameters: required_params,
                        return_type: None,
                        start_position: fallback_position,
                        end_position: fallback_position,
                        receiver: receiver.clone(),
                    }),
                });
            }
        }

        synthetic_calls
    }

    /// Check if a given name is a waiter
    ///
    /// This is useful for language extractors to quickly determine
    /// if a method call should be processed as a waiter.
    pub fn is_waiter(&self, name: &str) -> bool {
        self.service_index.waiter_lookup.contains_key(name)
    }

    /// Get required parameters for an operation from the service index
    fn get_required_parameters(&self, service_name: &str, operation_name: &str) -> Vec<Parameter> {
        let mut parameters = Vec::new();

        if let Some(service_def) = self.service_index.services.get(service_name) {
            if let Some(operation) = service_def.operations.get(operation_name) {
                if let Some(input_ref) = &operation.input {
                    if let Some(input_shape) = service_def.shapes.get(&input_ref.shape) {
                        if let Some(required_params) = &input_shape.required {
                            for (position, param_name) in required_params.iter().enumerate() {
                                // Create a generic parameter - language extractors can customize this
                                parameters.push(Parameter::Keyword {
                                    name: param_name.clone(),
                                    value: ParameterValue::Unresolved("<required>".to_string()),
                                    position,
                                    type_annotation: None,
                                });
                            }
                        }
                    }
                }
            }
        }

        parameters
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extraction::sdk_model::{
        Operation, SdkServiceDefinition, ServiceMetadata, ServiceMethodRef, Shape, ShapeReference,
    };
    use std::collections::HashMap;

    fn create_test_service_index() -> ServiceModelIndex {
        let mut services = HashMap::new();
        let mut waiter_lookup = HashMap::new();

        // Create a mock EC2 service with DescribeInstances operation
        let mut operations = HashMap::new();
        let mut shapes = HashMap::new();

        // Create input shape with required parameters
        let input_shape_members = HashMap::new();

        let input_shape = Shape {
            type_name: "structure".to_string(),
            members: input_shape_members,
            required: Some(vec!["InstanceIds".to_string()]),
        };

        shapes.insert("DescribeInstancesRequest".to_string(), input_shape);

        let operation = Operation {
            name: "DescribeInstances".to_string(),
            input: Some(ShapeReference {
                shape: "DescribeInstancesRequest".to_string(),
            }),
        };

        operations.insert("DescribeInstances".to_string(), operation);

        let service_def = SdkServiceDefinition {
            version: Some("2.0".to_string()),
            metadata: ServiceMetadata {
                api_version: "2016-11-15".to_string(),
                service_id: "EC2".to_string(),
            },
            operations,
            shapes,
        };

        services.insert("ec2".to_string(), service_def);

        // Add waiter lookup entry
        waiter_lookup.insert(
            "InstanceTerminated".to_string(),
            vec![ServiceMethodRef {
                service_name: "ec2".to_string(),
                operation_name: "DescribeInstances".to_string(),
            }],
        );

        ServiceModelIndex {
            services,
            method_lookup: HashMap::new(),
            waiter_lookup,
        }
    }

    #[test]
    fn test_resolve_unknown_waiter() {
        let service_index = create_test_service_index();
        let resolver = WaiterResolver::new(&service_index);

        // Test with unknown waiter - should return empty vec for operations
        assert!(!resolver.is_waiter("UnknownWaiter"));
    }

    #[test]
    fn test_create_synthetic_calls() {
        let service_index = create_test_service_index();
        let resolver = WaiterResolver::new(&service_index);

        let waiter_info = WaiterCallInfo {
            waiter_name: "InstanceTerminated".to_string(),
            parameters: vec![Parameter::Keyword {
                name: "InstanceIds".to_string(),
                value: ParameterValue::Resolved("['i-1234567890abcdef0']".to_string()),
                position: 0,
                type_annotation: None,
            }],
            start_position: (10, 5),
            end_position: (10, 25),
            receiver: Some("ec2_client".to_string()),
            possible_services: None,
        };

        let calls = resolver.create_synthetic_calls(&waiter_info);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "DescribeInstances");
        assert_eq!(calls[0].possible_services, vec!["ec2"]);

        let metadata = calls[0].metadata.as_ref().unwrap();
        assert_eq!(metadata.parameters.len(), 1);
        assert_eq!(metadata.start_position, (10, 5));
        assert_eq!(metadata.receiver, Some("ec2_client".to_string()));
    }

    #[test]
    fn test_create_fallback_synthetic_calls() {
        let service_index = create_test_service_index();
        let resolver = WaiterResolver::new(&service_index);

        let calls = resolver.create_fallback_synthetic_calls(
            "InstanceTerminated",
            (5, 1),
            Some("client".to_string()),
            None,
        );

        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "DescribeInstances");

        let metadata = calls[0].metadata.as_ref().unwrap();
        assert_eq!(metadata.parameters.len(), 1);
        if let Parameter::Keyword { name, .. } = &metadata.parameters[0] {
            assert_eq!(name, "InstanceIds");
        } else {
            panic!("Expected Keyword parameter");
        }
        assert_eq!(metadata.start_position, (5, 1));
        assert_eq!(metadata.receiver, Some("client".to_string()));
    }

    #[test]
    fn test_is_waiter() {
        let service_index = create_test_service_index();
        let resolver = WaiterResolver::new(&service_index);

        assert!(resolver.is_waiter("InstanceTerminated"));
        assert!(!resolver.is_waiter("NotAWaiter"));
    }

    #[test]
    fn test_service_filtering() {
        let service_index = create_test_service_index();
        let resolver = WaiterResolver::new(&service_index);

        let waiter_info = WaiterCallInfo {
            waiter_name: "InstanceTerminated".to_string(),
            parameters: vec![],
            start_position: (1, 1),
            end_position: (1, 1),
            receiver: None,
            possible_services: Some(vec!["s3".to_string()]), // Wrong service
        };

        let calls = resolver.create_synthetic_calls(&waiter_info);
        assert_eq!(calls.len(), 0); // Should be filtered out
    }
}
