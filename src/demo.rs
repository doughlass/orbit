use serde_json::json;
use std::collections::HashMap;

pub type DemoData = HashMap<&'static str, Vec<serde_json::Value>>;

pub fn all() -> DemoData {
    let mut data = DemoData::new();
    data.insert("ec2-instances", ec2_instances());
    data.insert("route53-hosted-zones", route53_hosted_zones());
    data.insert("cloudfront-distributions", cloudfront_distributions());
    data
}

pub fn load(keys: &[&str]) -> (DemoData, String) {
    let full = all();
    let select_all = keys == ["all"];
    let mut first = String::from("ec2-instances");
    let mut data = DemoData::new();
    let mut found = false;
    for (&k, v) in &full {
        if select_all || keys.contains(&k) {
            if !found {
                first = k.to_string();
                found = true;
            }
            data.insert(k, v.clone());
        }
    }
    (data, first)
}

fn ec2_instances() -> Vec<serde_json::Value> {
    vec![
        json!({"Tags":{"Name":"web-prod-01"},"InstanceId":"i-0a1b2c3d4e5f6g7h8","State":"running","InstanceType":"t3.medium","AvailabilityZone":"eu-west-1a","PublicIpAddress":"54.217.35.129","PrivateIpAddress":"10.0.1.10"}),
        json!({"Tags":{"Name":"web-prod-02"},"InstanceId":"i-1b2c3d4e5f6g7h8i9","State":"running","InstanceType":"t3.medium","AvailabilityZone":"eu-west-1b","PublicIpAddress":"52.49.89.201","PrivateIpAddress":"10.0.1.11"}),
        json!({"Tags":{"Name":"web-prod-03"},"InstanceId":"i-2c3d4e5f6g7h8i9j0","State":"stopped","InstanceType":"t3.medium","AvailabilityZone":"eu-west-1a","PublicIpAddress":"-","PrivateIpAddress":"10.0.1.12"}),
        json!({"Tags":{"Name":"api-gateway-01"},"InstanceId":"i-3d4e5f6g7h8i9j0k1","State":"running","InstanceType":"c6g.xlarge","AvailabilityZone":"eu-west-1c","PublicIpAddress":"34.247.151.88","PrivateIpAddress":"10.0.2.15"}),
        json!({"Tags":{"Name":"api-gateway-02"},"InstanceId":"i-4e5f6g7h8i9j0k1l2","State":"running","InstanceType":"c6g.xlarge","AvailabilityZone":"eu-west-1a","PublicIpAddress":"3.250.119.55","PrivateIpAddress":"10.0.2.16"}),
        json!({"Tags":{"Name":"bastion"},"InstanceId":"i-5f6g7h8i9j0k1l2m3","State":"running","InstanceType":"t4g.nano","AvailabilityZone":"eu-west-1b","PublicIpAddress":"63.35.16.42","PrivateIpAddress":"10.0.0.5"}),
        json!({"Tags":{"Name":"redis-cache-01"},"InstanceId":"i-6g7h8i9j0k1l2m3n4","State":"running","InstanceType":"r6g.large","AvailabilityZone":"eu-west-1a","PublicIpAddress":"-","PrivateIpAddress":"10.0.3.20"}),
        json!({"Tags":{"Name":"redis-cache-02"},"InstanceId":"i-7h8i9j0k1l2m3n4o5","State":"running","InstanceType":"r6g.large","AvailabilityZone":"eu-west-1c","PublicIpAddress":"-","PrivateIpAddress":"10.0.3.21"}),
        json!({"Tags":{"Name":"batch-worker-01"},"InstanceId":"i-8i9j0k1l2m3n4o5p6","State":"terminated","InstanceType":"m6i.2xlarge","AvailabilityZone":"eu-west-1b","PublicIpAddress":"-","PrivateIpAddress":"10.0.4.30"}),
        json!({"Tags":{"Name":"batch-worker-02"},"InstanceId":"i-9j0k1l2m3n4o5p6q7","State":"pending","InstanceType":"m6i.2xlarge","AvailabilityZone":"eu-west-1a","PublicIpAddress":"-","PrivateIpAddress":"10.0.4.31"}),
        json!({"Tags":{"Name":"ml-training-gpu"},"InstanceId":"i-0k1l2m3n4o5p6q7r8","State":"stopped","InstanceType":"p4d.24xlarge","AvailabilityZone":"eu-west-1c","PublicIpAddress":"-","PrivateIpAddress":"10.0.5.50"}),
        json!({"Tags":{"Name":"ci-runner-01"},"InstanceId":"i-1l2m3n4o5p6q7r8s9","State":"running","InstanceType":"c7g.4xlarge","AvailabilityZone":"eu-west-1a","PublicIpAddress":"18.202.77.213","PrivateIpAddress":"10.0.6.10"}),
        json!({"Tags":{"Name":"ci-runner-02"},"InstanceId":"i-2m3n4o5p6q7r8s9t0","State":"running","InstanceType":"c7g.4xlarge","AvailabilityZone":"eu-west-1b","PublicIpAddress":"18.203.88.194","PrivateIpAddress":"10.0.6.11"}),
        json!({"Tags":{"Name":"monitoring"},"InstanceId":"i-3n4o5p6q7r8s9t0u1","State":"stopping","InstanceType":"t3.small","AvailabilityZone":"eu-west-1a","PublicIpAddress":"-","PrivateIpAddress":"10.0.7.5"}),
        json!({"Tags":{"Name":"windows-build"},"InstanceId":"i-4o5p6q7r8s9t0u1v2","State":"running","InstanceType":"m6a.4xlarge","AvailabilityZone":"eu-west-1c","PublicIpAddress":"3.248.112.77","PrivateIpAddress":"10.0.8.22"}),
    ]
}

fn route53_hosted_zones() -> Vec<serde_json::Value> {
    vec![
        json!({"Name":"example.com.","Id":"/hostedzone/Z0123456789ABCDEF0","Config":{"PrivateZone":false},"ResourceRecordSetCount":14}),
        json!({"Name":"staging.example.com.","Id":"/hostedzone/Z1123456789ABCDEF1","Config":{"PrivateZone":false},"ResourceRecordSetCount":8}),
        json!({"Name":"dev.example.com.","Id":"/hostedzone/Z2123456789ABCDEF2","Config":{"PrivateZone":false},"ResourceRecordSetCount":5}),
        json!({"Name":"internal.corp.","Id":"/hostedzone/Z3123456789ABCDEF3","Config":{"PrivateZone":true},"ResourceRecordSetCount":42}),
        json!({"Name":"shared-services.internal.","Id":"/hostedzone/Z4123456789ABCDEF4","Config":{"PrivateZone":true},"ResourceRecordSetCount":23}),
    ]
}

fn cloudfront_distributions() -> Vec<serde_json::Value> {
    vec![
        json!({"Id":"E1ABCDEFGHIJKL","Aliases":"example.com, www.example.com","Origins":"api-origin (elb), assets-origin (s3)","DomainName":"d1234.cloudfront.net","Status":"Deployed","Enabled":true}),
        json!({"Id":"E2ABCDEFGHIJKL","Aliases":"cdn.example.com","Origins":"media-bucket (s3)","DomainName":"d5678.cloudfront.net","Status":"Deployed","Enabled":true}),
        json!({"Id":"E3ABCDEFGHIJKL","Aliases":"-","Origins":"api-prod (elb)","DomainName":"d9012.cloudfront.net","Status":"InProgress","Enabled":true}),
        json!({"Id":"E4ABCDEFGHIJKL","Aliases":"staging.example.com","Origins":"staging-alb (elb)","DomainName":"d3456.cloudfront.net","Status":"Deployed","Enabled":false}),
    ]
}
